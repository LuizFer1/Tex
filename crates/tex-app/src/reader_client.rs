use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    CloseRequestResponse, ComponentHandle, LogicalSize, Model, ModelNotify, ModelRc, ModelTracker,
    PhysicalPosition, SharedString, StyledText, VecModel,
};
use tex_platform::{
    lock_foreground, open_path, replace_file, reveal_in_folder, round_corners, take_foreground,
    work_area_under_cursor,
};
use tex_reader::{Body, Document, Encoding, Kind, LineIndex, LoadError, Stamp, blocks, probe};

use crate::frame;
use crate::reader::{
    BlockRow, CACHED_PAGES, LinkTarget, Lookup, MAX_HITS, Outside, PAGE_LINES, PageCache,
    SaveCheck, block_rows, display_line, display_lines, encoding_index, find_rows, line_count,
    link_target, longest, outside_change, reader_size, same_file, save_check, search_text,
    step_hit, wrap_by_default,
};
use crate::{ReaderBlock, ReaderWindow, Windows, placement};

thread_local! {
    static READERS: RefCell<Vec<Reader>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

/// An open reader window. Dropping it closes the window, the page channel and so the worker.
struct Reader {
    id: u64,
    path: PathBuf,
    window: ReaderWindow,
    lines: Option<Rc<LinesModel>>,
    cancel: Arc<AtomicBool>,
    load: u64,
    document: Option<Document>,
    index: Option<Arc<LineIndex>>,
    stamp: Option<Stamp>,
    markdown: bool,
    stale: bool,
    search_rows: Arc<Vec<String>>,
    search_cancel: Option<Arc<AtomicBool>>,
    search_id: u64,
    hits: Vec<usize>,
    hit: Option<usize>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(cancel) = &self.search_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

enum Event {
    Binary,
    Failed(String),
    Preview {
        lines: Vec<String>,
        encoding: Encoding,
    },
    Markdown {
        rows: Vec<StyledRow>,
        count: usize,
        document: Document,
        search_rows: Vec<String>,
    },
    Text {
        lines: Vec<String>,
        document: Document,
        search_rows: Vec<String>,
    },
    Indexed {
        count: usize,
        encoding: Encoding,
        pages: Sender<usize>,
        index: Arc<LineIndex>,
        stamp: Stamp,
    },
    Page {
        page: usize,
        lines: Vec<String>,
    },
}

/// Opens `path` in a reader window, or brings its open window to the front, then hides the popup.
pub fn open(windows: &Windows, path: &Path) {
    lock_foreground(false);
    let existing = READERS.with_borrow(|readers| {
        readers
            .iter()
            .find(|reader| same_file(&reader.path, path))
            .map(|reader| reader.window.clone_strong())
    });
    let is_new = existing.is_none();
    let Some(window) = existing.or_else(|| create(windows, path)) else {
        return;
    };
    crate::apply_theme(windows);
    present(&window, is_new);
    if let Some(popup) = windows.popup.upgrade() {
        crate::hide_popup(&popup);
    }
}

/// Runs `f` on every open reader window, outside the registry borrow.
pub fn each(mut f: impl FnMut(&ReaderWindow)) {
    let windows: Vec<ReaderWindow> = READERS.with_borrow(|readers| {
        readers
            .iter()
            .map(|reader| reader.window.clone_strong())
            .collect()
    });
    for window in &windows {
        f(window);
    }
}

fn create(windows: &Windows, path: &Path) -> Option<ReaderWindow> {
    let window = ReaderWindow::new().ok()?;
    let id = NEXT_ID.replace(NEXT_ID.get() + 1);
    let name = path
        .file_name()
        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy());
    window.set_file_name(name.as_ref().into());
    window.set_folder(
        path.parent()
            .map(|folder| folder.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into(),
    );
    window.set_wrap(wrap_by_default(path));
    wire(&window, windows, id, path);
    let cancel = Arc::new(AtomicBool::new(false));
    READERS.with_borrow_mut(|readers| {
        readers.push(Reader {
            id,
            path: path.to_path_buf(),
            window: window.clone_strong(),
            lines: None,
            cancel: cancel.clone(),
            load: 0,
            document: None,
            index: None,
            stamp: None,
            markdown: false,
            stale: false,
            search_rows: Arc::new(Vec::new()),
            search_cancel: None,
            search_id: 0,
            hits: Vec::new(),
            hit: None,
        })
    });
    spawn(id, 0, path.to_path_buf(), cancel);
    Some(window)
}

fn present(window: &ReaderWindow, is_new: bool) {
    if is_new && let Some(area) = work_area_under_cursor() {
        let (width, height) = reader_size(&area);
        window.window().set_size(LogicalSize::new(width, height));
        let (x, y) = placement::centered_origin(&area, width, height);
        window.window().set_position(PhysicalPosition::new(x, y));
    }
    if window.show().is_err() {
        return;
    }
    if window.window().is_minimized() {
        window.window().set_minimized(false);
    }
    if let Some(hwnd) = crate::window_hwnd(window.window()) {
        round_corners(hwnd);
        take_foreground(hwnd);
    }
}

fn wire(window: &ReaderWindow, windows: &Windows, id: u64, path: &Path) {
    frame::wire_frame!(window);
    window.window().on_close_requested({
        let window = window.as_weak();
        move || {
            let Some(window) = window.upgrade() else {
                return CloseRequestResponse::HideWindow;
            };
            if window.get_editing() {
                set_document_text(id, window.get_edit_text().to_string());
            }
            if document_dirty(id) {
                window.set_asking_close(true);
                return CloseRequestResponse::KeepWindowShown;
            }
            let _ = slint::invoke_from_event_loop(move || close(id));
            CloseRequestResponse::HideWindow
        }
    });
    window.window().on_winit_window_event(move |_, event| {
        if matches!(event, WindowEvent::Focused(true)) {
            let _ = slint::invoke_from_event_loop(move || check_outside(id));
        }
        EventResult::Propagate
    });
    window.on_edit_requested(move || start_edit(id));
    window.on_edit_finished(move || finish_edit(id));
    window.on_save_requested(move || {
        save(id, false, false);
    });
    window.on_banner_action(move |action| match action {
        1 => {
            save(id, true, false);
        }
        2 => reload(id),
        3 => {
            save(id, true, true);
        }
        _ => {}
    });
    window.on_banner_dismissed({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade() {
                window.set_banner(0);
            }
        }
    });
    window.on_close_choice({
        let window = window.as_weak();
        move |choice| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match choice {
                0 => {
                    window.set_asking_close(false);
                    if save(id, false, false) {
                        let _ = slint::invoke_from_event_loop(move || close(id));
                    }
                }
                1 => {
                    let _ = slint::invoke_from_event_loop(move || close(id));
                }
                _ => window.set_asking_close(false),
            }
        }
    });
    window.on_search_edited(move |query| search(id, query.to_string()));
    window.on_search_step(move |forward| search_step(id, forward));
    window.on_search_closed(move || search_closed(id));
    window.on_link_clicked({
        let windows = windows.clone();
        let file = path.to_path_buf();
        move |url| follow_link(&windows, &file, &url)
    });
}

fn close(id: u64) {
    let reader = READERS.with_borrow_mut(|readers| {
        let at = readers.iter().position(|reader| reader.id == id)?;
        Some(readers.remove(at))
    });
    if let Some(reader) = reader {
        let _ = reader.window.hide();
    }
}

fn follow_link(windows: &Windows, file: &Path, url: &str) {
    match link_target(file, url) {
        LinkTarget::Web(url) => {
            let _ = open_path(&url);
        }
        LinkTarget::Read(path) if !path.is_dir() => open(windows, &path),
        LinkTarget::Read(path) | LinkTarget::Reveal(path) => {
            let _ = reveal_in_folder(&path.to_string_lossy());
        }
        LinkTarget::Ignore => {}
    }
}

/// Probes and loads `path` off the UI thread; a large file then stays open to serve pages.
fn spawn(id: u64, load: u64, path: PathBuf, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let send = move |event: Event| {
            let _ = slint::invoke_from_event_loop(move || deliver(id, load, event));
        };
        let probed = match probe(&path, &[]) {
            Ok(probed) => probed,
            Err(LoadError::Io(error)) => return send(Event::Failed(error.to_string())),
            Err(LoadError::Binary | LoadError::Unsupported) => return send(Event::Binary),
        };
        if probed.large() {
            send(Event::Preview {
                lines: probed
                    .preview_lines()
                    .iter()
                    .map(|line| display_line(line))
                    .collect(),
                encoding: probed.encoding,
            });
        }
        let markdown = probed.kind == Kind::Markdown;
        let document = match probed.load(&cancel) {
            Ok(Some(document)) => document,
            Ok(None) => return,
            Err(LoadError::Io(error)) => return send(Event::Failed(error.to_string())),
            Err(LoadError::Binary | LoadError::Unsupported) => return send(Event::Binary),
        };
        let built = match &document.body {
            Body::Text(text) => Some(build(text, markdown)),
            Body::Lines(_) => None,
        };
        match built {
            Some((Built::Markdown { rows, count }, search_rows)) => send(Event::Markdown {
                rows,
                count,
                document,
                search_rows,
            }),
            Some((Built::Text { lines }, search_rows)) => send(Event::Text {
                lines,
                document,
                search_rows,
            }),
            None => {
                let Document {
                    body: Body::Lines(index),
                    stamp,
                    ..
                } = document
                else {
                    return;
                };
                serve_pages(&path, Arc::new(index), stamp, &send);
            }
        }
    });
}

/// What a text document shows, built off the UI thread when loading and on it after an edit.
enum Built {
    Markdown { rows: Vec<StyledRow>, count: usize },
    Text { lines: Vec<String> },
}

/// The rows to show for `text` and the plain text of each, which the search compares.
fn build(text: &str, markdown: bool) -> (Built, Vec<String>) {
    if markdown {
        let rows = block_rows(&blocks(text));
        let search_rows = rows.iter().map(search_text).collect();
        let rows = rows.into_iter().map(StyledRow::new).collect();
        let count = line_count(text);
        (Built::Markdown { rows, count }, search_rows)
    } else {
        let lines = display_lines(text);
        (
            Built::Text {
                lines: lines.clone(),
            },
            lines,
        )
    }
}

fn show_built(window: &ReaderWindow, built: Built) {
    match built {
        Built::Markdown { rows, count } => {
            let blocks: Vec<ReaderBlock> = rows.into_iter().map(reader_block).collect();
            window.set_blocks(ModelRc::new(VecModel::from(blocks)));
            window.set_markdown(true);
            window.set_line_count(count as i32);
        }
        Built::Text { lines } => {
            window.set_markdown(false);
            window.set_longest(longest(&lines) as i32);
            window.set_line_count(lines.len() as i32);
            window.set_lines(strings(lines));
        }
    }
}

fn serve_pages(path: &Path, index: Arc<LineIndex>, stamp: Stamp, send: &impl Fn(Event)) {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return send(Event::Failed(error.to_string())),
    };
    let (pages, requests) = mpsc::channel();
    send(Event::Indexed {
        count: index.len(),
        encoding: index.encoding(),
        pages,
        index: index.clone(),
        stamp,
    });
    while let Ok(page) = requests.recv() {
        let first = page * PAGE_LINES;
        let lines = index
            .read_lines(&mut file, first..first + PAGE_LINES)
            .unwrap_or_default();
        send(Event::Page { page, lines });
    }
}

fn deliver(id: u64, load: u64, event: Event) {
    let Some(window) = READERS.with_borrow(|readers| {
        readers
            .iter()
            .find(|reader| reader.id == id && reader.load == load)
            .map(|reader| reader.window.clone_strong())
    }) else {
        return;
    };
    match event {
        Event::Binary => close(id),
        Event::Failed(reason) => {
            window.set_error(reason.into());
            window.set_state(3);
        }
        Event::Preview { lines, encoding } => {
            window.set_markdown(false);
            window.set_longest(longest(&lines) as i32);
            window.set_lines(strings(lines));
            window.set_encoding(encoding_index(encoding));
            window.set_state(2);
        }
        Event::Markdown {
            rows,
            count,
            document,
            search_rows,
        } => {
            let encoding = document.encoding;
            store_document(id, document, true, search_rows);
            show_built(&window, Built::Markdown { rows, count });
            window.set_encoding(encoding_index(encoding));
            window.set_state(1);
        }
        Event::Text {
            lines,
            document,
            search_rows,
        } => {
            let encoding = document.encoding;
            store_document(id, document, false, search_rows);
            show_built(&window, Built::Text { lines });
            window.set_encoding(encoding_index(encoding));
            window.set_state(1);
        }
        Event::Indexed {
            count,
            encoding,
            pages,
            index,
            stamp,
        } => {
            let model = Rc::new(LinesModel::new(count, pages));
            with_reader(id, |reader| {
                reader.lines = Some(model.clone());
                reader.index = Some(index);
                reader.stamp = Some(stamp);
                reader.markdown = false;
            });
            window.set_markdown(false);
            window.set_lines(ModelRc::from(model));
            window.set_line_count(count as i32);
            window.set_encoding(encoding_index(encoding));
            window.set_state(1);
        }
        Event::Page { page, lines } => {
            let widest = longest(&lines) as i32;
            if widest > window.get_longest() {
                window.set_longest(widest);
            }
            let model = READERS.with_borrow(|readers| {
                readers
                    .iter()
                    .find(|reader| reader.id == id)
                    .and_then(|reader| reader.lines.clone())
            });
            if let Some(model) = model {
                model.page_loaded(page, lines);
            }
        }
    }
}

fn store_document(id: u64, document: Document, markdown: bool, search_rows: Vec<String>) {
    with_reader(id, |reader| {
        reader.stamp = Some(document.stamp);
        reader.document = Some(document);
        reader.markdown = markdown;
        reader.stale = false;
        reader.search_rows = Arc::new(search_rows);
    });
}

fn with_reader<T>(id: u64, f: impl FnOnce(&mut Reader) -> T) -> Option<T> {
    READERS.with_borrow_mut(|readers| readers.iter_mut().find(|reader| reader.id == id).map(f))
}

fn window_of(id: u64) -> Option<ReaderWindow> {
    READERS.with_borrow(|readers| {
        readers
            .iter()
            .find(|reader| reader.id == id)
            .map(|reader| reader.window.clone_strong())
    })
}

/// Records what the editor holds in the document, remembering that the view no longer shows it.
fn record_text(reader: &mut Reader, text: String) {
    if let Some(document) = &mut reader.document
        && matches!(&document.body, Body::Text(current) if *current != text)
    {
        document.set_text(text);
        reader.stale = true;
    }
}

fn set_document_text(id: u64, text: String) {
    with_reader(id, |reader| record_text(reader, text));
}

fn document_dirty(id: u64) -> bool {
    with_reader(id, |reader| {
        reader
            .document
            .as_ref()
            .is_some_and(|document| document.dirty)
    })
    .unwrap_or(false)
}

/// Clears the search state of a reader whose rows changed.
fn reset_search(reader: &mut Reader) {
    if let Some(cancel) = reader.search_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
    reader.search_id += 1;
    reader.hits.clear();
    reader.hit = None;
}

fn clear_search_view(window: &ReaderWindow) {
    window.set_search_busy(false);
    window.set_hit_count(0);
    window.set_hit_index(-1);
    window.set_current_row(-1);
}

/// Starts a search for `query` on a thread that stops when a newer search, a reload or the close cancels it.
fn search(id: u64, query: String) {
    let Some(window) = window_of(id) else {
        return;
    };
    let started = with_reader(id, |reader| {
        reset_search(reader);
        if query.trim().is_empty() {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        reader.search_cancel = Some(cancel.clone());
        Some((
            reader.search_id,
            cancel,
            reader.search_rows.clone(),
            reader.index.clone(),
            reader.path.clone(),
        ))
    })
    .flatten();
    clear_search_view(&window);
    let Some((search_id, cancel, rows, index, path)) = started else {
        return;
    };
    window.set_search_busy(true);
    std::thread::spawn(move || {
        let hits = match index {
            Some(index) => find_in_file(&index, &path, &query, &cancel),
            None => find_rows(&rows, &query, 0, &cancel),
        };
        if let Some(hits) = hits {
            let _ = slint::invoke_from_event_loop(move || search_done(id, search_id, hits));
        }
    });
}

/// Searches an indexed file a block of lines at a time; `None` when cancelled or unreadable.
fn find_in_file(
    index: &LineIndex,
    path: &Path,
    query: &str,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    let mut file = File::open(path).ok()?;
    let step = PAGE_LINES * 8;
    let mut hits = Vec::new();
    let mut first = 0;
    while first < index.len() && hits.len() < MAX_HITS {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let chunk = index.read_lines(&mut file, first..first + step).ok()?;
        if chunk.is_empty() {
            break;
        }
        hits.extend(find_rows(&chunk, query, first, cancel)?);
        first += chunk.len();
    }
    hits.truncate(MAX_HITS);
    Some(hits)
}

fn search_done(id: u64, search_id: u64, hits: Vec<usize>) {
    let Some(window) = window_of(id) else {
        return;
    };
    let first = with_reader(id, |reader| {
        if reader.search_id != search_id {
            return None;
        }
        reader.search_cancel = None;
        let first = hits.first().copied();
        reader.hit = first.map(|_| 0);
        reader.hits = hits;
        Some((reader.hits.len(), first))
    })
    .flatten();
    let Some((count, first)) = first else {
        return;
    };
    window.set_search_busy(false);
    window.set_hit_count(count as i32);
    if let Some(row) = first {
        window.set_hit_index(0);
        window.set_current_row(row as i32);
        window.invoke_jump_to(row as i32);
    }
}

/// Moves to the next or previous hit; with none, opens the search bar.
fn search_step(id: u64, forward: bool) {
    let Some(window) = window_of(id) else {
        return;
    };
    let step = with_reader(id, |reader| {
        reader.hit = step_hit(reader.hits.len(), reader.hit, forward);
        reader.hit.map(|at| (at, reader.hits[at]))
    })
    .flatten();
    match step {
        Some((at, row)) => {
            window.set_hit_index(at as i32);
            window.set_current_row(row as i32);
            window.invoke_jump_to(row as i32);
        }
        None if !window.get_searching() => window.set_searching(true),
        None => {}
    }
}

fn search_closed(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    with_reader(id, reset_search);
    clear_search_view(&window);
}

/// Starts editing: the raw text goes to the editor, or a banner says why it cannot.
fn start_edit(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    if window.get_editing() {
        return;
    }
    let target = with_reader(id, |reader| match (&reader.document, &reader.index) {
        (Some(document), _) => match &document.body {
            Body::Text(text) if document.editable() => Some(Some(text.clone())),
            _ => Some(None),
        },
        (None, Some(_)) => Some(None),
        (None, None) => None,
    })
    .flatten();
    match target {
        Some(Some(text)) => {
            window.set_edit_original(text.clone().into());
            window.set_edit_text(text.into());
            window.set_banner(0);
            window.set_editing(true);
        }
        Some(None) => window.set_banner(1),
        None => {}
    }
}

/// Leaves the editor: the typed text goes into the document and the view is rebuilt when it changed.
fn finish_edit(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let text = window.get_edit_text().to_string();
    let result = with_reader(id, |reader| {
        record_text(reader, text);
        let document = reader.document.as_ref()?;
        let dirty = document.dirty;
        let Body::Text(current) = &document.body else {
            return Some((None, dirty));
        };
        if !reader.stale {
            return Some((None, dirty));
        }
        let (built, search_rows) = build(current, reader.markdown);
        reader.search_rows = Arc::new(search_rows);
        reader.stale = false;
        reset_search(reader);
        Some((Some(built), dirty))
    })
    .flatten();
    if let Some((built, dirty)) = result {
        if let Some(built) = built {
            show_built(&window, built);
            clear_search_view(&window);
        }
        window.set_dirty(dirty);
    }
    window.set_editing(false);
}

enum SaveOutcome {
    Skip,
    Ask(i32),
    Failed(String),
    Saved,
}

/// Saves the document; returns whether it was written. Asks through the banner first when the file
/// changed outside (unless `overwrite`) or characters would be lost (unless `lossy`).
fn save(id: u64, overwrite: bool, lossy: bool) -> bool {
    let Some(window) = window_of(id) else {
        return false;
    };
    let typed = window.get_editing().then(|| window.get_edit_text());
    let outcome = with_reader(id, |reader| {
        if let Some(text) = typed {
            record_text(reader, text.to_string());
        }
        let Some(document) = reader.document.as_mut() else {
            return SaveOutcome::Skip;
        };
        if !document.editable() {
            return SaveOutcome::Skip;
        }
        let changed = match Stamp::read(&reader.path) {
            Ok(stamp) => document.changed(stamp),
            Err(_) => false,
        };
        match save_check(changed, document.fits(), overwrite, lossy) {
            SaveCheck::AskOutside => return SaveOutcome::Ask(2),
            SaveCheck::AskLossy => return SaveOutcome::Ask(5),
            SaveCheck::Write => {}
        }
        let Some(bytes) = document.save_bytes() else {
            return SaveOutcome::Skip;
        };
        match replace_file(&reader.path, &bytes) {
            Err(error) => SaveOutcome::Failed(error.to_string()),
            Ok(()) => {
                let stamp = Stamp::read(&reader.path).unwrap_or(document.stamp);
                document.saved(stamp);
                reader.stamp = Some(stamp);
                SaveOutcome::Saved
            }
        }
    })
    .unwrap_or(SaveOutcome::Skip);
    match outcome {
        SaveOutcome::Skip => false,
        SaveOutcome::Ask(banner) => {
            window.set_banner(banner);
            false
        }
        SaveOutcome::Failed(detail) => {
            window.set_banner_detail(detail.into());
            window.set_banner(4);
            false
        }
        SaveOutcome::Saved => {
            window.set_dirty(false);
            if window.get_editing() {
                window.set_edit_original(window.get_edit_text());
            }
            window.set_banner(0);
            window.invoke_saved();
            true
        }
    }
}

/// Drops what was loaded and loads the file again; events of the old load are ignored.
fn reload(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let started = with_reader(id, |reader| {
        reader.cancel.store(true, Ordering::Relaxed);
        reader.cancel = Arc::new(AtomicBool::new(false));
        reader.load += 1;
        reader.document = None;
        reader.index = None;
        reader.lines = None;
        reader.stamp = None;
        reader.search_rows = Arc::new(Vec::new());
        reset_search(reader);
        (reader.load, reader.path.clone(), reader.cancel.clone())
    });
    window.set_state(0);
    window.set_editing(false);
    window.set_dirty(false);
    window.set_banner(0);
    window.set_searching(false);
    clear_search_view(&window);
    if let Some((load, path, cancel)) = started {
        spawn(id, load, path, cancel);
    }
}

/// On focus: compares the file with what was loaded and reloads, warns or does nothing.
fn check_outside(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let typing = window.get_editing() && window.get_edit_text() != window.get_edit_original();
    let verdict = with_reader(id, |reader| {
        let known = reader.stamp?;
        let current = match Stamp::read(&reader.path) {
            Ok(stamp) => Some(stamp),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(_) => return None,
        };
        let dirty = typing
            || reader
                .document
                .as_ref()
                .is_some_and(|document| document.dirty);
        Some(outside_change(known, current, dirty))
    })
    .flatten();
    match verdict {
        Some(Outside::Reload) => reload(id),
        Some(Outside::Conflict) => window.set_banner(2),
        Some(Outside::Deleted) => window.set_banner(3),
        Some(Outside::Nothing) | None => {}
    }
}

fn strings(lines: Vec<String>) -> ModelRc<SharedString> {
    let lines: Vec<SharedString> = lines.into_iter().map(SharedString::from).collect();
    ModelRc::new(VecModel::from(lines))
}

fn styled(markup: &str) -> StyledText {
    StyledText::from_markdown(markup).unwrap_or_else(|_| StyledText::from_plain_text(markup))
}

/// A block row with its Markdown already parsed, so the worker does it instead of the UI thread.
struct StyledRow {
    row: BlockRow,
    text: StyledText,
    cells: Vec<StyledText>,
}

impl StyledRow {
    fn new(row: BlockRow) -> StyledRow {
        StyledRow {
            text: styled(&row.markup),
            cells: row.cells.iter().map(|cell| styled(cell)).collect(),
            row,
        }
    }
}

fn reader_block(styled: StyledRow) -> ReaderBlock {
    let StyledRow { row, text, cells } = styled;
    ReaderBlock {
        kind: row.kind.index(),
        level: row.level as i32,
        marker: row.marker.into(),
        text,
        plain: row.plain.into(),
        cells: ModelRc::new(VecModel::from(cells)),
        header: row.header,
    }
}

/// The lines of a large file, read a page at a time by the worker as the list shows them.
struct LinesModel {
    count: usize,
    cache: RefCell<PageCache>,
    requests: Sender<usize>,
    notify: ModelNotify,
}

impl LinesModel {
    fn new(count: usize, requests: Sender<usize>) -> LinesModel {
        LinesModel {
            count,
            cache: RefCell::new(PageCache::new(CACHED_PAGES)),
            requests,
            notify: ModelNotify::default(),
        }
    }

    fn page_loaded(&self, page: usize, lines: Vec<String>) {
        let first = page * PAGE_LINES;
        let last = (first + lines.len()).min(self.count);
        self.cache.borrow_mut().insert(page, lines);
        for row in first..last {
            self.notify.row_changed(row);
        }
    }
}

impl Model for LinesModel {
    type Data = SharedString;

    fn row_count(&self) -> usize {
        self.count
    }

    fn row_data(&self, row: usize) -> Option<SharedString> {
        if row >= self.count {
            return None;
        }
        match self.cache.borrow_mut().get(row) {
            Lookup::Hit(line) => Some(line.into()),
            Lookup::Miss { request } => {
                if let Some(page) = request {
                    let _ = self.requests.send(page);
                }
                Some(SharedString::new())
            }
        }
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
