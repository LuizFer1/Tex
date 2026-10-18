use std::ops::Range;

use slint::{ModelRc, VecModel};
use tex_platform::{open_path, write_text};
use tex_reader::{Body, TEXT_LIMIT, channel, job, run};
use tex_reader::{Catalog, Document, Extension, Input, LIMITS, RunError, Tool, ToolOutput};

use super::edit::apply_text;
use super::model::strings;
use super::{Banner, reload_catalog, show_banner, window_of, with_reader};
use crate::reader::{display_lines, longest};
use crate::settings::extensions_dir;
use crate::shortcut::Shortcut;
use crate::tools::{
    Context, KeyProblem, Loaded, Reason, describe, note, resolve, selection_range, splice,
};
use crate::{ProblemRow, ReaderWindow, ToolRow};

/// A tool of the catalog that applies to the open file, with the shortcut it got.
#[derive(Clone)]
pub(super) struct ToolEntry {
    extension: Extension,
    tool: Tool,
    key: Result<Option<Shortcut>, KeyProblem>,
}

fn entries(catalog: &Catalog, path: &std::path::Path) -> Vec<ToolEntry> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let tools = catalog.tools_for(ext.as_deref());
    let requested: Vec<(String, Option<String>)> = tools
        .iter()
        .map(|(_, tool)| (tool.title.clone(), tool.shortcut.clone()))
        .collect();
    tools
        .into_iter()
        .zip(resolve(&requested))
        .map(|((extension, tool), key)| ToolEntry {
            extension: extension.clone(),
            tool: tool.clone(),
            key,
        })
        .collect()
}

/// What the open file can offer a tool right now.
fn context(id: u64, window: &ReaderWindow) -> Context {
    let editing = window.get_editing();
    let selection = editing
        && selection_range(
            &window.get_edit_text(),
            window.get_sel_anchor(),
            window.get_sel_cursor(),
        )
        .is_some();
    let (loaded, replaceable) = with_reader(id, |reader| match &reader.document {
        Some(document) => (
            match document.body {
                Body::Text(_) => Loaded::Text,
                Body::Lines(_) => Loaded::TooLarge,
            },
            document.editable() || document.dirty,
        ),
        None if reader.viewer && reader.stamp.is_some_and(|stamp| stamp.len <= TEXT_LIMIT) => {
            (Loaded::NotText, false)
        }
        None => (Loaded::TooLarge, false),
    })
    .unwrap_or((Loaded::TooLarge, false));
    Context {
        editing,
        selection,
        loaded,
        replaceable,
    }
}

/// Makes sure the reader has its tools, from the catalog it holds.
fn ensure_tools(id: u64) {
    with_reader(id, |reader| {
        if reader.tools.is_none() {
            reader.tools = Some(entries(&reader.catalog, &reader.path));
        }
    });
}

/// Reloads the catalog and shows the menu with the tools of this file and what is wrong with the folder.
pub(super) fn tools_requested(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let catalog = reload_catalog();
    let tools = with_reader(id, |reader| {
        reader.catalog = catalog.clone();
        let tools = entries(&catalog, &reader.path);
        reader.tools = Some(tools.clone());
        tools
    })
    .unwrap_or_default();
    let context = context(id, &window);
    let rows: Vec<ToolRow> = tools
        .iter()
        .map(|entry| {
            let note = note(entry.tool.input, entry.tool.output, context, &entry.key);
            ToolRow {
                title: entry.tool.title.as_str().into(),
                shortcut: match &entry.key {
                    Ok(Some(shortcut)) => shortcut.display().into(),
                    _ => "".into(),
                },
                enabled: note.enabled,
                note_kind: note.kind,
                note_arg: note.arg.into(),
            }
        })
        .collect();
    let mut problems: Vec<ProblemRow> = catalog
        .broken
        .iter()
        .map(|broken| ProblemRow {
            kind: 0,
            name: broken
                .folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
            detail: broken.error.as_str().into(),
        })
        .collect();
    problems.extend(
        catalog
            .viewer_conflicts()
            .into_iter()
            .map(|(ext, name)| ProblemRow {
                kind: 1,
                name: name.into(),
                detail: ext.into(),
            }),
    );
    window.set_tools(ModelRc::new(VecModel::from(rows)));
    window.set_problems(ModelRc::new(VecModel::from(problems)));
    window.set_tools_open(true);
}

/// Runs the tool whose shortcut is `text` + modifiers; returns whether some tool owns that shortcut.
pub(super) fn tool_key(id: u64, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(pressed) = Shortcut::from_slint(text, ctrl, alt, shift, false) else {
        return false;
    };
    ensure_tools(id);
    let found = with_reader(id, |reader| {
        reader
            .tools
            .as_ref()?
            .iter()
            .position(|entry| matches!(&entry.key, Ok(Some(shortcut)) if *shortcut == pressed))
    })
    .flatten();
    let Some(index) = found else {
        return false;
    };
    run_tool(id, index);
    true
}

/// Runs tool `index` of the reader's list on the document or the selection, off the UI thread.
pub(super) fn run_tool(id: u64, index: usize) {
    let Some(window) = window_of(id) else {
        return;
    };
    let context = context(id, &window);
    let editing = window.get_editing();
    let edit_text = window.get_edit_text().to_string();
    let selection = (window.get_sel_anchor(), window.get_sel_cursor());
    let prepared = with_reader(id, |reader| {
        if reader.tool_cancel.is_some() {
            return None;
        }
        let entry = reader.tools.as_ref()?.get(index)?.clone();
        if !note(entry.tool.input, entry.tool.output, context, &entry.key).enabled {
            return None;
        }
        let base = if editing {
            edit_text
        } else {
            match &reader.document {
                Some(Document {
                    body: Body::Text(text),
                    ..
                }) => text.clone(),
                _ => return None,
            }
        };
        let range = match entry.tool.input {
            Input::Document => 0..base.len(),
            Input::Selection => selection_range(&base, selection.0, selection.1)?,
        };
        let (cancel, inbox) = channel();
        reader.run += 1;
        reader.tool_cancel = Some(cancel);
        let process = job(
            &entry.extension,
            &entry.tool.command,
            base[range.clone()].as_bytes().to_vec(),
            &reader.path,
        );
        Some((entry, base, range, reader.run, inbox, process))
    })
    .flatten();
    let Some((entry, base, range, run_id, inbox, process)) = prepared else {
        return;
    };
    window.set_banner_detail(entry.tool.title.as_str().into());
    show_banner(&window, Banner::Running);
    std::thread::spawn(move || {
        let result = run(&process, LIMITS, inbox);
        let _ = slint::invoke_from_event_loop(move || {
            finished(id, run_id, entry, base, range, result);
        });
    });
}

/// Stops the running tool; its result then clears the banner.
pub(super) fn cancel_tool(id: u64) {
    let cancel = with_reader(id, |reader| reader.tool_cancel.clone()).flatten();
    if let Some(cancel) = cancel {
        cancel.cancel();
    }
}

fn clear_running(window: &ReaderWindow) {
    if window.get_banner() == Banner::Running as i32 {
        show_banner(window, Banner::None);
    }
}

fn failed(window: &ReaderWindow, title: &str, reason: Reason) {
    window.set_banner_title(title.into());
    window.set_banner_reason(reason.kind);
    window.set_banner_detail(reason.detail.into());
    show_banner(window, Banner::ToolFailed);
}

fn finished(
    id: u64,
    run_id: u64,
    entry: ToolEntry,
    base: String,
    range: Range<usize>,
    result: Result<String, RunError>,
) {
    let Some(window) = window_of(id) else {
        return;
    };
    let current = with_reader(id, |reader| {
        let current = reader.run == run_id;
        if current {
            reader.tool_cancel = None;
        }
        current
    });
    if current != Some(true) {
        return;
    }
    let title = entry.tool.title.as_str();
    let output = match result {
        Ok(output) => output,
        Err(RunError::Cancelled) => return clear_running(&window),
        Err(error) => return failed(&window, title, describe(&error, LIMITS.time.as_secs())),
    };
    match entry.tool.output {
        ToolOutput::Replace => replace(id, &window, title, &base, range, &output),
        ToolOutput::Show => {
            let lines = display_lines(&output);
            with_reader(id, |reader| reader.output = output);
            window.set_output_title(title.into());
            window.set_output_longest(longest(&lines) as i32);
            window.set_output_lines(strings(lines));
            window.set_output_open(true);
            clear_running(&window);
        }
        ToolOutput::None => {
            clear_running(&window);
            window.invoke_done();
        }
    }
}

/// Puts the output in place of `range` of `base`, unless the text changed while the tool ran.
fn replace(
    id: u64,
    window: &ReaderWindow,
    title: &str,
    base: &str,
    range: Range<usize>,
    output: &str,
) {
    let editing = window.get_editing();
    let current = if editing {
        Some(window.get_edit_text().to_string())
    } else {
        with_reader(id, |reader| match &reader.document {
            Some(Document {
                body: Body::Text(text),
                ..
            }) => Some(text.clone()),
            _ => None,
        })
        .flatten()
    };
    if current.as_deref() != Some(base) {
        return failed(window, title, Reason::new(7, ""));
    }
    let text = splice(base, range, output);
    clear_running(window);
    if editing {
        window.set_edit_text(text.into());
    } else {
        apply_text(id, window, text);
    }
}

/// Opens the extensions folder in the file manager, creating it first.
pub(super) fn open_extensions() {
    let dir = extensions_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = open_path(&dir.to_string_lossy());
}

/// Copies the whole output of the last tool, not only the lines the panel shows.
pub(super) fn output_copy(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let copied = with_reader(id, |reader| write_text(&reader.output).is_ok());
    if copied == Some(true) {
        window.invoke_copied();
    }
}

pub(super) fn output_closed(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    with_reader(id, |reader| reader.output = String::new());
    window.set_output_open(false);
    window.set_output_lines(ModelRc::default());
}
