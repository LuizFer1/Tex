use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tex_reader::LineIndex;

use super::{Reader, window_of, with_reader};
use crate::ReaderWindow;
use crate::reader::{MAX_HITS, PAGE_LINES, find_rows, step_hit};

/// Clears the search state of a reader whose rows changed.
pub(super) fn reset_search(reader: &mut Reader) {
    if let Some(cancel) = reader.search_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
    reader.search_id += 1;
    reader.hits.clear();
    reader.hit = None;
}

pub(super) fn clear_search_view(window: &ReaderWindow) {
    window.set_search_busy(false);
    window.set_hit_count(0);
    window.set_hit_index(-1);
    window.set_current_row(-1);
}

/// Starts a search for `query` on a thread that stops when a newer search, a reload or the close cancels it.
pub(super) fn search(id: u64, query: String) {
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
pub(super) fn search_step(id: u64, forward: bool) {
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

pub(super) fn search_closed(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    with_reader(id, reset_search);
    clear_search_view(&window);
}
