use std::cell::{Cell, RefCell};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use slint::{
    CloseRequestResponse, ComponentHandle, LogicalSize, Model, ModelNotify, ModelRc, ModelTracker,
    PhysicalPosition, SharedString, StyledText, VecModel,
};
use tex_platform::{
    lock_foreground, open_path, reveal_in_folder, round_corners, take_foreground,
    work_area_under_cursor,
};
use tex_reader::{Body, Document, Encoding, Kind, LineIndex, LoadError, blocks, probe};

use crate::frame;
use crate::reader::{
    BlockRow, CACHED_PAGES, LinkTarget, Lookup, PAGE_LINES, PageCache, block_rows, display_line,
    display_lines, encoding_index, line_count, link_target, longest, reader_size, same_file,
    wrap_by_default,
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
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        encoding: Encoding,
    },
    Text {
        lines: Vec<String>,
        encoding: Encoding,
    },
    Indexed {
        count: usize,
        encoding: Encoding,
        pages: Sender<usize>,
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
        })
    });
    spawn(id, path.to_path_buf(), cancel);
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
    window.window().on_close_requested(move || {
        let _ = slint::invoke_from_event_loop(move || close(id));
        CloseRequestResponse::HideWindow
    });
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
fn spawn(id: u64, path: PathBuf, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let send = move |event: Event| {
            let _ = slint::invoke_from_event_loop(move || deliver(id, event));
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
        let Document { body, encoding, .. } = match probed.load(&cancel) {
            Ok(Some(document)) => document,
            Ok(None) => return,
            Err(LoadError::Io(error)) => return send(Event::Failed(error.to_string())),
            Err(LoadError::Binary | LoadError::Unsupported) => return send(Event::Binary),
        };
        match body {
            Body::Text(text) if markdown => send(Event::Markdown {
                rows: block_rows(&blocks(&text))
                    .into_iter()
                    .map(StyledRow::new)
                    .collect(),
                count: line_count(&text),
                encoding,
            }),
            Body::Text(text) => send(Event::Text {
                lines: display_lines(&text),
                encoding,
            }),
            Body::Lines(index) => serve_pages(&path, index, &send),
        }
    });
}

fn serve_pages(path: &Path, index: LineIndex, send: &impl Fn(Event)) {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return send(Event::Failed(error.to_string())),
    };
    let (pages, requests) = mpsc::channel();
    send(Event::Indexed {
        count: index.len(),
        encoding: index.encoding(),
        pages,
    });
    while let Ok(page) = requests.recv() {
        let first = page * PAGE_LINES;
        let lines = index
            .read_lines(&mut file, first..first + PAGE_LINES)
            .unwrap_or_default();
        send(Event::Page { page, lines });
    }
}

fn deliver(id: u64, event: Event) {
    if matches!(event, Event::Binary) {
        close(id);
        return;
    }
    let Some(window) = READERS.with_borrow(|readers| {
        readers
            .iter()
            .find(|reader| reader.id == id)
            .map(|reader| reader.window.clone_strong())
    }) else {
        return;
    };
    match event {
        Event::Binary => {}
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
            encoding,
        } => {
            let blocks: Vec<ReaderBlock> = rows.into_iter().map(reader_block).collect();
            window.set_blocks(ModelRc::new(VecModel::from(blocks)));
            window.set_markdown(true);
            window.set_line_count(count as i32);
            window.set_encoding(encoding_index(encoding));
            window.set_state(1);
        }
        Event::Text { lines, encoding } => {
            window.set_markdown(false);
            window.set_longest(longest(&lines) as i32);
            window.set_line_count(lines.len() as i32);
            window.set_lines(strings(lines));
            window.set_encoding(encoding_index(encoding));
            window.set_state(1);
        }
        Event::Indexed {
            count,
            encoding,
            pages,
        } => {
            let model = Rc::new(LinesModel::new(count, pages));
            READERS.with_borrow_mut(|readers| {
                if let Some(reader) = readers.iter_mut().find(|reader| reader.id == id) {
                    reader.lines = Some(model.clone());
                }
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
