use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tex_platform::replace_file;
use tex_reader::{Body, Stamp};

use super::search::{clear_search_view, reset_search};
use super::{Banner, Reader, build, show_banner, show_built, spawn, window_of, with_reader};
use crate::reader::{Outside, SaveCheck, outside_change, save_check};

/// Records what the editor holds in the document, remembering that the view no longer shows it.
fn record_text(reader: &mut Reader, text: String) {
    if let Some(document) = &mut reader.document
        && document.set_text(text)
    {
        reader.stale = true;
    }
}

pub(super) fn set_document_text(id: u64, text: String) {
    with_reader(id, |reader| record_text(reader, text));
}

pub(super) fn document_dirty(id: u64) -> bool {
    with_reader(id, |reader| {
        reader
            .document
            .as_ref()
            .is_some_and(|document| document.dirty)
    })
    .unwrap_or(false)
}

/// Starts editing: the raw text goes to the editor, or a banner says why it cannot.
pub(super) fn start_edit(id: u64) {
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
            show_banner(&window, Banner::None);
            window.set_editing(true);
        }
        Some(None) => show_banner(&window, Banner::TooLarge),
        None => {}
    }
}

/// Leaves the editor: the typed text goes into the document and the view is rebuilt when it changed.
pub(super) fn finish_edit(id: u64) {
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
    AskOutside,
    AskLossy,
    Failed(String),
    Saved,
}

/// Saves the document; returns whether it was written. Asks through the banner first when the file
/// changed outside (unless `overwrite`) or characters would be lost (unless `lossy`).
pub(super) fn save(id: u64, overwrite: bool, lossy: bool) -> bool {
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
            SaveCheck::AskOutside => return SaveOutcome::AskOutside,
            SaveCheck::AskLossy => return SaveOutcome::AskLossy,
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
        SaveOutcome::AskOutside => {
            show_banner(&window, Banner::Outside);
            false
        }
        SaveOutcome::AskLossy => {
            show_banner(&window, Banner::Lossy);
            false
        }
        SaveOutcome::Failed(detail) => {
            window.set_banner_detail(detail.into());
            show_banner(&window, Banner::SaveFailed);
            false
        }
        SaveOutcome::Saved => {
            window.set_dirty(false);
            if window.get_editing() {
                window.set_edit_original(window.get_edit_text());
            }
            show_banner(&window, Banner::None);
            window.invoke_saved();
            true
        }
    }
}

/// Drops what was loaded and loads the file again; events of the old load are ignored.
pub(super) fn reload(id: u64) {
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
    show_banner(&window, Banner::None);
    window.set_searching(false);
    clear_search_view(&window);
    if let Some((load, path, cancel)) = started {
        spawn(id, load, path, cancel);
    }
}

/// On focus: compares the file with what was loaded and reloads, warns or does nothing.
pub(super) fn check_outside(id: u64) {
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
        Some(Outside::Conflict) => show_banner(&window, Banner::Outside),
        Some(Outside::Deleted) => show_banner(&window, Banner::Deleted),
        Some(Outside::Nothing) | None => {}
    }
}
