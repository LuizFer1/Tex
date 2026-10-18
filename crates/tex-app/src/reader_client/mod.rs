mod edit;
mod model;
mod search;

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    CloseRequestResponse, ComponentHandle, LogicalSize, ModelRc, PhysicalPosition, VecModel,
};
use tex_platform::{
    lock_foreground, open_path, reveal_in_folder, round_corners, take_foreground,
    work_area_under_cursor,
};
use tex_reader::{Body, Document, Encoding, Kind, LineIndex, LoadError, Stamp, blocks, probe};

use crate::frame;
use crate::reader::{
    LinkTarget, PAGE_LINES, block_rows, display_line, display_lines, encoding_index, line_count,
    link_target, longest, reader_size, same_file, search_text, wrap_by_default,
};
use crate::{ReaderBlock, ReaderWindow, Windows, placement};

use edit::{check_outside, finish_edit, reload, save, start_edit, unsaved};
use model::{LinesModel, StyledRow, reader_block, strings};
use search::{search, search_closed, search_step};

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
            if unsaved(id, &window) {
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
    window.on_banner_action({
        let window = window.as_weak();
        move |action| match action {
            1 => {
                save(id, true, false);
            }
            2 => match window.upgrade() {
                Some(window) if unsaved(id, &window) => window.set_asking_reload(true),
                _ => reload(id),
            },
            3 => {
                save(id, true, true);
            }
            _ => {}
        }
    });
    window.on_reload_choice({
        let window = window.as_weak();
        move |yes| {
            if let Some(window) = window.upgrade() {
                window.set_asking_reload(false);
            }
            if yes {
                reload(id);
            }
        }
    });
    window.on_banner_dismissed({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade() {
                show_banner(&window, Banner::None);
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
            if let Some(model) = with_reader(id, |reader| reader.lines.clone()).flatten() {
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

/// The banners of `reader.slint`, by the code its `banner` property takes.
#[derive(Clone, Copy)]
enum Banner {
    None = 0,
    TooLarge = 1,
    Outside = 2,
    Deleted = 3,
    SaveFailed = 4,
    Lossy = 5,
}

fn show_banner(window: &ReaderWindow, banner: Banner) {
    window.set_banner(banner as i32);
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
