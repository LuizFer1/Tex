use std::error::Error;
use std::fs;
use std::io;
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tex_clipboard::{
    CaptureSettings, ClipFacts, Content, Decision, EditError, HistoryFilter, KEY_LEN, Key,
    NewEntry, OpenError, Retention, Store, Summary, dib_to_rgba, encode_png, fits, new_key,
    should_capture, thumbnail,
};
use tex_platform::{Clip, ClipFormats, Payload, protect, unprotect};

pub const PAGE_SIZE: usize = 200;
const SEARCH_LIMIT: usize = 500;
const THUMBNAIL_SIZE: u32 = 96;
const DATABASE: &str = "clipboard.db";
const KEY: &str = "clipboard.key";
const HISTORY_FILES: [&str; 4] = [DATABASE, "clipboard.db-wal", "clipboard.db-shm", KEY];

#[derive(Debug, Clone, PartialEq)]
pub struct ClientSettings {
    pub capture: CaptureSettings,
    pub retention: Retention,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Configure(ClientSettings),
    Captured(Clip),
    Page {
        filter: HistoryFilter,
        before: Option<i64>,
    },
    Search {
        query: String,
        filter: HistoryFilter,
        generation: u64,
    },
    Content(i64),
    SetFavorite(i64, bool),
    Rename(i64, Option<String>),
    EditText(i64, String),
    Delete(i64),
    Clear,
    Usage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Unavailable(String),
    Page { rows: Vec<Summary>, append: bool },
    SearchResults { generation: u64, rows: Vec<Summary> },
    Added(Summary),
    Changed(Summary),
    Removed(Vec<i64>),
    Content(i64, Option<Content>),
    EditRefused(i64, EditError),
    Usage(u64),
}

#[derive(Clone)]
pub struct ClipboardClient {
    commands: Sender<Command>,
    generation: Arc<AtomicU64>,
    capture: Arc<Mutex<CaptureSettings>>,
}

impl ClipboardClient {
    /// Opens (or creates) the history in `folder` on a worker thread. `on_event` runs there.
    pub fn start(
        folder: PathBuf,
        settings: ClientSettings,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let capture = Arc::new(Mutex::new(settings.capture.clone()));
        thread::spawn({
            let generation = Arc::clone(&generation);
            move || run(&receiver, &folder, settings, generation, &on_event)
        });
        Self {
            commands,
            generation,
            capture,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Hands new settings to the worker and to [`ClipboardClient::wanted`].
    pub fn configure(&self, settings: ClientSettings) {
        *self.capture.lock().unwrap_or_else(PoisonError::into_inner) = settings.capture.clone();
        self.send(Command::Configure(settings));
    }

    /// What the clipboard watcher should read for these formats under the current settings.
    pub fn wanted(&self, formats: &ClipFormats) -> Option<Payload> {
        payload_for(
            formats,
            &self.capture.lock().unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Starts a search and cancels the one running; returns its generation.
    pub fn search(&self, query: String, filter: HistoryFilter) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.send(Command::Search {
            query,
            filter,
            generation,
        });
        generation
    }

    /// Cancels the running search, if any.
    pub fn cancel_search(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn history_folder() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                .join("AppData")
                .join("Local")
        });
    base.join("Tex")
}

/// Opens the history, then answers commands. Waits on the channel only until the oldest
/// entry expires, so the time policy runs without a timer.
fn run(
    receiver: &Receiver<Command>,
    folder: &Path,
    settings: ClientSettings,
    generation: Arc<AtomicU64>,
    on_event: &dyn Fn(Event),
) {
    let store = match open_store(folder) {
        Ok(store) => store,
        Err(reason) => {
            on_event(Event::Unavailable(reason.clone()));
            while receiver.recv().is_ok() {
                on_event(Event::Unavailable(reason.clone()));
            }
            return;
        }
    };
    let mut worker = Worker {
        store,
        settings,
        generation,
    };
    report(worker.retain(on_event));
    loop {
        let first = match worker.deadline() {
            Some(deadline) => {
                let wait = u64::try_from(deadline.saturating_sub(now_ms())).unwrap_or(0) + 1;
                match receiver.recv_timeout(Duration::from_millis(wait)) {
                    Ok(command) => command,
                    Err(RecvTimeoutError::Timeout) => {
                        let result = worker.retain(on_event);
                        if result.is_ok() {
                            continue;
                        }
                        report(result);
                        match receiver.recv() {
                            Ok(command) => command,
                            Err(_) => return,
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match receiver.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        for command in coalesce(iter::once(first).chain(receiver.try_iter())) {
            handle(&mut worker, command, on_event);
        }
    }
}

struct Worker {
    store: Store,
    settings: ClientSettings,
    generation: Arc<AtomicU64>,
}

impl Worker {
    fn deadline(&self) -> Option<i64> {
        self.store
            .next_expiry(self.settings.retention)
            .ok()
            .flatten()
    }

    /// Applies the retention policy and reports what it removed and the new usage.
    fn retain(&mut self, emit: &dyn Fn(Event)) -> Result<(), Box<dyn Error>> {
        let removed = self
            .store
            .apply_retention(self.settings.retention, now_ms())?;
        if !removed.is_empty() {
            emit(Event::Removed(removed));
        }
        emit(Event::Usage(self.store.usage()?));
        Ok(())
    }

    fn changed(&self, id: i64, emit: &dyn Fn(Event)) -> Result<(), Box<dyn Error>> {
        if let Some(summary) = self.store.summary(id)? {
            emit(Event::Changed(summary));
        }
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The key in `clipboard.key`, protected by DPAPI. A key that does not unprotect to 32 bytes,
/// or a history without a key, sets the old files aside as `.bad` and makes a new key.
fn load_key(folder: &Path) -> Result<Key, String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    match fs::read(folder.join(KEY)) {
        Ok(protected) => {
            let bytes = unprotect(&protected)
                .ok()
                .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes.as_slice()).ok());
            if let Some(bytes) = bytes {
                return Ok(Key::from_bytes(bytes));
            }
            set_aside(folder);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if folder.join(DATABASE).exists() {
                set_aside(folder);
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    create_key(folder).map_err(|error| error.to_string())
}

fn create_key(folder: &Path) -> io::Result<Key> {
    let key = new_key();
    let temporary = folder.join("clipboard.key.tmp");
    fs::write(&temporary, protect(key.as_bytes())?)?;
    fs::rename(&temporary, folder.join(KEY))?;
    Ok(key)
}

fn set_aside(folder: &Path) {
    for name in HISTORY_FILES {
        let path = folder.join(name);
        if path.exists() {
            let _ = fs::rename(&path, folder.join(format!("{name}.bad")));
        }
    }
}

/// A history the key does not open is set aside and started over.
fn open_store(folder: &Path) -> Result<Store, String> {
    let path = folder.join(DATABASE);
    match Store::open(&path, load_key(folder)?) {
        Err(OpenError::WrongKey) => {
            set_aside(folder);
            Store::open(&path, load_key(folder)?).map_err(|error| error.to_string())
        }
        result => result.map_err(|error| error.to_string()),
    }
}

/// Keeps only the newest Search, at its place; every other command runs in order.
fn coalesce(commands: impl Iterator<Item = Command>) -> Vec<Command> {
    let mut commands: Vec<Command> = commands.collect();
    let is_search = |command: &Command| matches!(command, Command::Search { .. });
    if let Some(last) = commands.iter().rposition(is_search) {
        let mut index = 0;
        commands.retain(|command| {
            let keep = index == last || !is_search(command);
            index += 1;
            keep
        });
    }
    commands
}

fn facts_from_clip(clip: &Clip) -> ClipFacts<'_> {
    ClipFacts {
        from_tex: clip.from_tex,
        excluded: clip.excluded,
        history_disallowed: clip.history_disallowed,
        owner_exe: clip.owner_exe.as_deref(),
        has_text: clip.text.as_deref().is_some_and(|text| !text.is_empty()),
        has_image: clip.dib.as_deref().is_some_and(|dib| !dib.is_empty()),
    }
}

/// Applies the capture rules to the formats alone, so paused, opted-out, ignored and Tex's
/// own copies read nothing, and an image is read only when it is the one captured.
fn payload_for(formats: &ClipFormats, settings: &CaptureSettings) -> Option<Payload> {
    let facts = ClipFacts {
        from_tex: formats.from_tex,
        excluded: formats.excluded,
        history_disallowed: formats.history_disallowed,
        owner_exe: formats.owner_exe.as_deref(),
        has_text: formats.text,
        has_image: formats.image,
    };
    match should_capture(&facts, settings) {
        Decision::Text => Some(Payload::Text),
        Decision::Image => Some(Payload::Image),
        Decision::Skip(_) => None,
    }
}

/// What a clipboard change adds to the history, when the capture rules and the size limit
/// accept it. Images are measured as PNG.
fn entry_from_clip(clip: Clip, settings: &CaptureSettings) -> Option<NewEntry> {
    let decision = should_capture(&facts_from_clip(&clip), settings);
    match decision {
        Decision::Text => {
            let text = clip.text?;
            fits(text.len() as u64, settings).then_some(NewEntry::Text(text))
        }
        Decision::Image => {
            let image = dib_to_rgba(clip.dib.as_deref()?)?;
            let png = encode_png(&image);
            if !fits(png.len() as u64, settings) {
                return None;
            }
            Some(NewEntry::Image {
                thumbnail: encode_png(&thumbnail(&image, THUMBNAIL_SIZE)),
                png,
                width: image.width,
                height: image.height,
            })
        }
        Decision::Skip(_) => None,
    }
}

fn report(result: Result<(), Box<dyn Error>>) {
    if let Err(error) = result {
        eprintln!("clipboard history: {error}");
    }
}

fn handle(worker: &mut Worker, command: Command, emit: &dyn Fn(Event)) {
    report(apply(worker, command, emit));
}

fn apply(
    worker: &mut Worker,
    command: Command,
    emit: &dyn Fn(Event),
) -> Result<(), Box<dyn Error>> {
    match command {
        Command::Configure(settings) => {
            worker.settings = settings;
            worker.retain(emit)?;
        }
        Command::Captured(clip) => {
            let Some(entry) = entry_from_clip(clip, &worker.settings.capture) else {
                return Ok(());
            };
            let summary = worker.store.insert(&entry, now_ms())?;
            emit(Event::Added(summary));
            worker.retain(emit)?;
        }
        Command::Page { filter, before } => {
            let rows = worker.store.page(filter, before, PAGE_SIZE)?;
            emit(Event::Page {
                rows,
                append: before.is_some(),
            });
        }
        Command::Search {
            query,
            filter,
            generation,
        } => {
            let current = Arc::clone(&worker.generation);
            let cancelled = move || current.load(Ordering::SeqCst) != generation;
            if cancelled() {
                return Ok(());
            }
            if let Some(rows) = worker
                .store
                .search(&query, filter, SEARCH_LIMIT, &cancelled)?
            {
                emit(Event::SearchResults { generation, rows });
            }
        }
        Command::Content(id) => {
            let content = worker.store.content(id).unwrap_or_else(|error| {
                report(Err(error.into()));
                None
            });
            emit(Event::Content(id, content));
        }
        Command::SetFavorite(id, favorite) => {
            worker.store.set_favorite(id, favorite)?;
            worker.changed(id, emit)?;
            worker.retain(emit)?;
        }
        Command::Rename(id, name) => {
            worker.store.rename(id, name.as_deref())?;
            worker.changed(id, emit)?;
        }
        Command::EditText(id, text) => {
            let max = worker.settings.capture.max_item_bytes;
            match worker.store.edit_text(id, &text, max)? {
                Ok(summary) => {
                    emit(Event::Changed(summary));
                    worker.retain(emit)?;
                }
                Err(error) => emit(Event::EditRefused(id, error)),
            }
        }
        Command::Delete(id) => {
            worker.store.delete(id)?;
            emit(Event::Removed(vec![id]));
            emit(Event::Usage(worker.store.usage()?));
        }
        Command::Clear => {
            let removed = worker.store.clear_non_favorites()?;
            if !removed.is_empty() {
                emit(Event::Removed(removed));
            }
            emit(Event::Usage(worker.store.usage()?));
        }
        Command::Usage => emit(Event::Usage(worker.store.usage()?)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use tex_clipboard::{DAY_MS, Kind, Preview, Rgba, decode_png, rgba_to_dib};
    use tex_platform::protect;

    use super::*;

    const MB: u64 = 1024 * 1024;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Folder {
            let path = std::env::temp_dir().join(format!(
                "tex-clipboard-client-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn settings() -> ClientSettings {
        ClientSettings {
            capture: CaptureSettings {
                paused: false,
                capture_text: true,
                capture_images: true,
                max_item_bytes: 10 * MB,
                respect_exclusion: true,
                ignored_apps: vec!["KeePass.exe".into()],
            },
            retention: Retention::Time { days: 3 },
        }
    }

    fn worker(settings: ClientSettings) -> Worker {
        Worker {
            store: Store::open_in_memory(new_key()).unwrap(),
            settings,
            generation: Arc::default(),
        }
    }

    fn run(worker: &mut Worker, command: Command) -> Vec<Event> {
        let events = RefCell::new(Vec::new());
        handle(worker, command, &|event| events.borrow_mut().push(event));
        events.into_inner()
    }

    fn text_clip(text: &str) -> Clip {
        Clip {
            text: Some(text.into()),
            owner_exe: Some("notepad.exe".into()),
            ..Clip::default()
        }
    }

    fn search(query: &str, generation: u64) -> Command {
        Command::Search {
            query: query.into(),
            filter: HistoryFilter::All,
            generation,
        }
    }

    fn added(events: &[Event]) -> Vec<&Summary> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Added(summary) => Some(summary),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn load_key_creates_and_reads_back_the_same_key() {
        let folder = Folder::new("same-key");
        let key = load_key(&folder.0).unwrap();
        assert!(folder.join("clipboard.key").exists());
        assert!(!folder.join("clipboard.key.tmp").exists());
        let again = load_key(&folder.0).unwrap();
        assert_eq!(key.as_bytes(), again.as_bytes());
    }

    #[test]
    fn database_without_key_is_set_aside() {
        let folder = Folder::new("no-key");
        fs::write(folder.join("clipboard.db"), b"old history").unwrap();
        fs::write(folder.join("clipboard.db-wal"), b"wal").unwrap();
        load_key(&folder.0).unwrap();
        assert!(!folder.join("clipboard.db").exists());
        assert!(!folder.join("clipboard.db-wal").exists());
        assert_eq!(
            fs::read(folder.join("clipboard.db.bad")).unwrap(),
            b"old history"
        );
        assert!(folder.join("clipboard.db-wal.bad").exists());
        assert!(folder.join("clipboard.key").exists());
    }

    #[test]
    fn garbage_key_is_set_aside() {
        let folder = Folder::new("garbage-key");
        fs::write(folder.join("clipboard.key"), b"garbage").unwrap();
        fs::write(folder.join("clipboard.db"), b"old history").unwrap();
        let key = load_key(&folder.0).unwrap();
        assert_eq!(
            fs::read(folder.join("clipboard.key.bad")).unwrap(),
            b"garbage"
        );
        assert!(folder.join("clipboard.db.bad").exists());
        assert!(!folder.join("clipboard.db").exists());
        assert_eq!(load_key(&folder.0).unwrap().as_bytes(), key.as_bytes());
    }

    #[test]
    fn key_of_the_wrong_size_is_set_aside() {
        let folder = Folder::new("short-key");
        fs::write(folder.join("clipboard.key"), protect(&[7; 16]).unwrap()).unwrap();
        load_key(&folder.0).unwrap();
        assert!(folder.join("clipboard.key.bad").exists());
    }

    #[test]
    fn database_of_another_key_starts_over() {
        let folder = Folder::new("wrong-key");
        {
            let mut store = open_store(&folder.0).unwrap();
            store.insert(&NewEntry::Text("old".into()), 1_000).unwrap();
        }
        let other = new_key();
        fs::write(
            folder.join("clipboard.key"),
            protect(other.as_bytes()).unwrap(),
        )
        .unwrap();
        let store = open_store(&folder.0).unwrap();
        assert!(store.page(HistoryFilter::All, None, 10).unwrap().is_empty());
        assert!(folder.join("clipboard.db.bad").exists());
        assert!(folder.join("clipboard.key.bad").exists());
        drop(store);
        assert_ne!(load_key(&folder.0).unwrap().as_bytes(), other.as_bytes());
    }

    #[test]
    fn coalesce_keeps_the_last_search_and_the_order_of_the_rest() {
        let commands = vec![
            search("a", 1),
            Command::Usage,
            search("ab", 2),
            Command::Delete(4),
            search("abc", 3),
            Command::Clear,
        ];
        assert_eq!(
            coalesce(commands.into_iter()),
            [
                Command::Usage,
                Command::Delete(4),
                search("abc", 3),
                Command::Clear
            ]
        );
        let commands = vec![Command::Content(1), Command::Usage];
        assert_eq!(coalesce(commands.clone().into_iter()), commands.as_slice());
    }

    #[test]
    fn facts_follow_the_clip() {
        let clip = Clip {
            text: Some("hello".into()),
            dib: Some(vec![1, 2, 3]),
            excluded: true,
            history_disallowed: true,
            from_tex: true,
            owner_exe: Some("KeePass.exe".into()),
        };
        let facts = facts_from_clip(&clip);
        assert!(facts.from_tex);
        assert!(facts.excluded);
        assert!(facts.history_disallowed);
        assert_eq!(facts.owner_exe, Some("KeePass.exe"));
        assert!(facts.has_text);
        assert!(facts.has_image);

        let empty = Clip {
            text: Some(String::new()),
            dib: Some(Vec::new()),
            ..Clip::default()
        };
        let facts = facts_from_clip(&empty);
        assert!(!facts.from_tex && !facts.excluded && !facts.history_disallowed);
        assert_eq!(facts.owner_exe, None);
        assert!(!facts.has_text);
        assert!(!facts.has_image);
    }

    fn client(settings: ClientSettings) -> (ClipboardClient, Receiver<Command>) {
        let (commands, receiver) = std::sync::mpsc::channel();
        let client = ClipboardClient {
            commands,
            generation: Arc::default(),
            capture: Arc::new(Mutex::new(settings.capture)),
        };
        (client, receiver)
    }

    fn formats(text: bool, image: bool) -> ClipFormats {
        ClipFormats {
            text,
            image,
            owner_exe: Some("EXCEL.EXE".into()),
            ..ClipFormats::default()
        }
    }

    #[test]
    fn reads_only_the_content_that_will_be_captured() {
        let mut capture = settings().capture;
        assert_eq!(
            payload_for(&formats(true, true), &capture),
            Some(Payload::Text)
        );
        assert_eq!(
            payload_for(&formats(false, true), &capture),
            Some(Payload::Image)
        );
        capture.capture_images = false;
        assert_eq!(payload_for(&formats(false, true), &capture), None);
        capture.capture_text = false;
        assert_eq!(payload_for(&formats(true, false), &capture), None);
        capture.capture_images = true;
        assert_eq!(
            payload_for(&formats(true, true), &capture),
            Some(Payload::Image)
        );
        assert_eq!(
            payload_for(&formats(false, false), &settings().capture),
            None
        );
    }

    #[test]
    fn skipped_copies_read_nothing() {
        let capture = settings().capture;
        let paused = CaptureSettings {
            paused: true,
            ..settings().capture
        };
        assert_eq!(payload_for(&formats(true, true), &paused), None);
        let ignored = ClipFormats {
            owner_exe: Some("keepass.exe".into()),
            ..formats(true, true)
        };
        assert_eq!(payload_for(&ignored, &capture), None);
        let excluded = ClipFormats {
            excluded: true,
            ..formats(true, true)
        };
        assert_eq!(payload_for(&excluded, &capture), None);
        let disallowed = ClipFormats {
            history_disallowed: true,
            ..formats(true, true)
        };
        assert_eq!(payload_for(&disallowed, &capture), None);
        let own = ClipFormats {
            from_tex: true,
            ..formats(true, true)
        };
        assert_eq!(payload_for(&own, &capture), None);
    }

    #[test]
    fn configure_changes_what_the_watcher_reads() {
        let (client, receiver) = client(settings());
        assert_eq!(client.wanted(&formats(true, false)), Some(Payload::Text));
        let mut paused = settings();
        paused.capture.paused = true;
        client.configure(paused.clone());
        assert_eq!(client.wanted(&formats(true, false)), None);
        assert_eq!(receiver.try_recv().unwrap(), Command::Configure(paused));
    }

    #[test]
    fn captured_text_is_added() {
        let mut worker = worker(settings());
        let events = run(&mut worker, Command::Captured(text_clip("olá\nmundo")));
        let rows = added(&events);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, Kind::Text);
        assert_eq!(rows[0].preview, Preview::Text("olá mundo".into()));
        assert_eq!(rows[0].size, "olá\nmundo".len() as u64);
        assert_eq!(events.last(), Some(&Event::Usage(rows[0].size)));
    }

    #[test]
    fn identical_copies_are_separate_rows() {
        let mut worker = worker(settings());
        run(&mut worker, Command::Captured(text_clip("same")));
        run(&mut worker, Command::Captured(text_clip("same")));
        let page = worker.store.page(HistoryFilter::All, None, 10).unwrap();
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn text_over_the_maximum_is_not_captured() {
        let mut settings = settings();
        settings.capture.max_item_bytes = 4;
        let mut worker = worker(settings);
        assert!(run(&mut worker, Command::Captured(text_clip("12345"))).is_empty());
        assert!(!added(&run(&mut worker, Command::Captured(text_clip("1234")))).is_empty());
    }

    #[test]
    fn skipped_clips_are_not_captured() {
        let mut worker = worker(settings());
        let mut ignored = text_clip("secret");
        ignored.owner_exe = Some("keepass.exe".into());
        let mut own = text_clip("again");
        own.from_tex = true;
        assert!(run(&mut worker, Command::Captured(ignored)).is_empty());
        assert!(run(&mut worker, Command::Captured(own)).is_empty());
        assert!(run(&mut worker, Command::Captured(Clip::default())).is_empty());
        assert!(
            worker
                .store
                .page(HistoryFilter::All, None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn captured_image_is_added_as_png_with_thumbnail() {
        let image = Rgba {
            width: 200,
            height: 100,
            pixels: vec![200; 200 * 100 * 4],
        };
        let clip = Clip {
            dib: Some(rgba_to_dib(&image)),
            ..Clip::default()
        };
        let mut worker = worker(settings());
        let events = run(&mut worker, Command::Captured(clip));
        let rows = added(&events);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, Kind::Image);
        assert_eq!((rows[0].width, rows[0].height), (Some(200), Some(100)));
        let Preview::Image(thumbnail) = &rows[0].preview else {
            panic!("image preview expected");
        };
        let thumbnail = decode_png(thumbnail).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (96, 48));
        let content = worker.store.content(rows[0].id).unwrap();
        let Some(Content::Image { png, .. }) = content else {
            panic!("image content expected");
        };
        assert_eq!(decode_png(&png), Some(image));
    }

    #[test]
    fn broken_image_is_not_captured() {
        let clip = Clip {
            dib: Some(vec![0; 10]),
            ..Clip::default()
        };
        let mut worker = worker(settings());
        assert!(run(&mut worker, Command::Captured(clip)).is_empty());
    }

    #[test]
    fn unfavoriting_an_expired_row_removes_it() {
        let mut worker = worker(settings());
        let old = worker
            .store
            .insert(&NewEntry::Text("old".into()), 1_000)
            .unwrap();
        worker.store.set_favorite(old.id, true).unwrap();
        let events = run(&mut worker, Command::SetFavorite(old.id, false));
        assert!(events.contains(&Event::Removed(vec![old.id])));
        assert!(worker.store.summary(old.id).unwrap().is_none());
    }

    #[test]
    fn favoriting_reports_the_changed_row() {
        let mut worker = worker(settings());
        let row = worker
            .store
            .insert(&NewEntry::Text("keep".into()), super::now_ms())
            .unwrap();
        let events = run(&mut worker, Command::SetFavorite(row.id, true));
        let Some(Event::Changed(changed)) = events.first() else {
            panic!("changed expected, got {events:?}");
        };
        assert!(changed.favorite);
        assert!(events.contains(&Event::Usage(0)));
    }

    #[test]
    fn rename_edit_delete_and_clear_report_their_rows() {
        let mut worker = worker(settings());
        let now = super::now_ms();
        let first = worker
            .store
            .insert(&NewEntry::Text("first".into()), now)
            .unwrap();
        let second = worker
            .store
            .insert(&NewEntry::Text("second".into()), now)
            .unwrap();
        worker.store.set_favorite(second.id, true).unwrap();

        let events = run(&mut worker, Command::Rename(first.id, Some("Nome".into())));
        assert!(
            matches!(&events[..], [Event::Changed(row)] if row.name.as_deref() == Some("Nome"))
        );

        let events = run(&mut worker, Command::EditText(first.id, "edited".into()));
        assert!(
            matches!(&events[0], Event::Changed(row) if row.preview == Preview::Text("edited".into()))
        );
        assert!(events.contains(&Event::Usage(6)));

        worker.settings.capture.max_item_bytes = 3;
        let events = run(&mut worker, Command::EditText(first.id, "toolong".into()));
        assert_eq!(events, [Event::EditRefused(first.id, EditError::TooLarge)]);

        let events = run(&mut worker, Command::Content(first.id));
        assert_eq!(
            events,
            [Event::Content(
                first.id,
                Some(Content::Text("edited".into()))
            )]
        );

        let third = worker
            .store
            .insert(&NewEntry::Text("third".into()), now)
            .unwrap();
        let events = run(&mut worker, Command::Delete(third.id));
        assert_eq!(events[0], Event::Removed(vec![third.id]));

        let events = run(&mut worker, Command::Clear);
        assert_eq!(events, [Event::Removed(vec![first.id]), Event::Usage(0)]);
        assert!(worker.store.summary(second.id).unwrap().is_some());
        assert_eq!(run(&mut worker, Command::Usage), [Event::Usage(0)]);
    }

    #[test]
    fn pages_append_only_when_continuing() {
        let mut worker = worker(settings());
        let now = super::now_ms();
        let ids: Vec<i64> = (0..3)
            .map(|index| {
                worker
                    .store
                    .insert(&NewEntry::Text(format!("row {index}")), now)
                    .unwrap()
                    .id
            })
            .collect();
        let events = run(
            &mut worker,
            Command::Page {
                filter: HistoryFilter::All,
                before: None,
            },
        );
        let [
            Event::Page {
                rows,
                append: false,
            },
        ] = &events[..]
        else {
            panic!("first page expected, got {events:?}");
        };
        assert_eq!(rows.len(), 3);
        let events = run(
            &mut worker,
            Command::Page {
                filter: HistoryFilter::All,
                before: Some(ids[1]),
            },
        );
        let [Event::Page { rows, append: true }] = &events[..] else {
            panic!("next page expected, got {events:?}");
        };
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [ids[0]]);
    }

    #[test]
    fn search_reports_its_generation_and_drops_cancelled_ones() {
        let mut worker = worker(settings());
        worker
            .store
            .insert(&NewEntry::Text("Ação rápida".into()), super::now_ms())
            .unwrap();
        worker
            .generation
            .store(2, std::sync::atomic::Ordering::SeqCst);
        let events = run(&mut worker, search("acao", 2));
        let [
            Event::SearchResults {
                generation: 2,
                rows,
            },
        ] = &events[..]
        else {
            panic!("results expected, got {events:?}");
        };
        assert_eq!(rows.len(), 1);
        assert!(run(&mut worker, search("acao", 1)).is_empty());
    }

    #[test]
    fn cancel_search_drops_the_running_search() {
        let (client, receiver) = client(settings());
        let generation = client.search("acao".into(), HistoryFilter::All);
        client.cancel_search();
        let mut worker = worker(settings());
        worker.generation = Arc::clone(&client.generation);
        worker
            .store
            .insert(&NewEntry::Text("Ação".into()), super::now_ms())
            .unwrap();
        let command = receiver.try_recv().unwrap();
        assert_eq!(command, search("acao", generation));
        assert!(run(&mut worker, command).is_empty());
    }

    #[test]
    fn configure_applies_the_new_retention() {
        let mut worker = worker(ClientSettings {
            retention: Retention::Time { days: 30 },
            ..settings()
        });
        let now = super::now_ms();
        let old = worker
            .store
            .insert(&NewEntry::Text("old".into()), now - 10 * DAY_MS)
            .unwrap();
        let events = run(&mut worker, Command::Configure(settings()));
        assert_eq!(events, [Event::Removed(vec![old.id]), Event::Usage(0)]);
        assert_eq!(worker.settings, settings());
    }

    #[test]
    fn retention_waits_until_the_oldest_row_expires() {
        let mut worker = worker(settings());
        assert_eq!(worker.deadline(), None);
        let row = worker
            .store
            .insert(&NewEntry::Text("x".into()), 5_000)
            .unwrap();
        assert_eq!(worker.deadline(), Some(5_000 + 3 * DAY_MS));
        worker.store.set_favorite(row.id, true).unwrap();
        assert_eq!(worker.deadline(), None);
        worker.settings.retention = Retention::Storage { limit_bytes: MB };
        worker.store.set_favorite(row.id, false).unwrap();
        assert_eq!(worker.deadline(), None);
    }

    #[test]
    fn client_runs_commands_on_its_thread() {
        let folder = Folder::new("client");
        let (sender, events) = std::sync::mpsc::channel();
        let client = ClipboardClient::start(folder.0.clone(), settings(), move |event| {
            let _ = sender.send(event);
        });
        let timeout = std::time::Duration::from_secs(10);
        assert_eq!(events.recv_timeout(timeout).unwrap(), Event::Usage(0));
        client.send(Command::Captured(text_clip("hello")));
        assert!(matches!(
            events.recv_timeout(timeout).unwrap(),
            Event::Added(_)
        ));
        assert_eq!(events.recv_timeout(timeout).unwrap(), Event::Usage(5));
        let first = client.search("hel".into(), HistoryFilter::All);
        let second = client.search("hell".into(), HistoryFilter::All);
        assert!(second > first);
        let last = loop {
            match events.recv_timeout(timeout).unwrap() {
                Event::SearchResults { generation, rows } => {
                    if generation == second {
                        break rows;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(last.len(), 1);
        drop(client);
    }
}
