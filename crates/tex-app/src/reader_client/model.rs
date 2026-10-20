use std::cell::RefCell;
use std::sync::mpsc::Sender;

use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, StyledText, VecModel};

use crate::ReaderBlock;
use crate::reader::{BlockRow, CACHED_PAGES, Lookup, PAGE_LINES, PageCache};

pub(super) fn strings(lines: Vec<String>) -> ModelRc<SharedString> {
    let lines: Vec<SharedString> = lines.into_iter().map(SharedString::from).collect();
    ModelRc::new(VecModel::from(lines))
}

fn styled(markup: &str) -> StyledText {
    StyledText::from_markdown(markup).unwrap_or_else(|_| StyledText::from_plain_text(markup))
}

/// A block row with its Markdown already parsed, so the worker does it instead of the UI thread.
pub(super) struct StyledRow {
    row: BlockRow,
    text: StyledText,
    cells: Vec<StyledText>,
}

impl StyledRow {
    pub(super) fn new(row: BlockRow) -> StyledRow {
        StyledRow {
            text: styled(&row.markup),
            cells: row.cells.iter().map(|cell| styled(cell)).collect(),
            row,
        }
    }
}

pub(super) fn reader_block(styled: StyledRow) -> ReaderBlock {
    let StyledRow { row, text, cells } = styled;
    ReaderBlock {
        kind: row.kind.index(),
        level: row.level as i32,
        marker: row.marker.into(),
        text,
        plain: row.plain.into(),
        cells: ModelRc::new(VecModel::from(cells)),
        header: row.header,
        src: row.src.into(),
        title: row.title.into(),
        ..ReaderBlock::default()
    }
}

/// The lines of a large file, read a page at a time by the worker as the list shows them.
pub(super) struct LinesModel {
    count: usize,
    cache: RefCell<PageCache>,
    requests: Sender<usize>,
    notify: ModelNotify,
}

impl LinesModel {
    pub(super) fn new(count: usize, requests: Sender<usize>) -> LinesModel {
        LinesModel {
            count,
            cache: RefCell::new(PageCache::new(CACHED_PAGES)),
            requests,
            notify: ModelNotify::default(),
        }
    }

    pub(super) fn page_loaded(&self, page: usize, lines: Vec<String>) {
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
