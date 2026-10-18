use std::ops::Range;

use tex_reader::{Input, RunError, ToolOutput};

use crate::shortcut::Shortcut;

/// The reader's own keys, which a tool can never take.
pub const READER_KEYS: [&str; 7] = [
    "ctrl+e",
    "ctrl+s",
    "ctrl+shift+e",
    "ctrl+shift+s",
    "ctrl+f",
    "shift+f3",
    "alt+z",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyProblem {
    Invalid,
    Reserved,
    Taken(String),
}

/// Whether the reader can see the shortcut: it needs Ctrl or Alt, and Ctrl+Alt only with a letter
/// or an F key, because AltGr arrives as Ctrl+Alt on Windows and would fire it while typing `@` or `€`.
fn fits_ctrl_alt(shortcut: &Shortcut) -> bool {
    if !(shortcut.ctrl || shortcut.alt) {
        return false;
    }
    let name = shortcut.key.name;
    let letter = name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic();
    let function = name.len() > 1
        && name.starts_with('f')
        && name[1..].bytes().all(|byte| byte.is_ascii_digit());
    !(shortcut.ctrl && shortcut.alt) || letter || function
}

/// The shortcut each tool gets, in order: a later tool asking for a key already taken loses it.
pub fn resolve(
    requested: &[(String, Option<String>)],
) -> Vec<Result<Option<Shortcut>, KeyProblem>> {
    let mut taken: Vec<(String, String)> = Vec::new();
    requested
        .iter()
        .map(|(title, text)| {
            let Some(text) = text else { return Ok(None) };
            let shortcut = Shortcut::parse(text).ok_or(KeyProblem::Invalid)?;
            let key = shortcut.canonical();
            if READER_KEYS.contains(&key.as_str()) {
                return Err(KeyProblem::Reserved);
            }
            if !fits_ctrl_alt(&shortcut) {
                return Err(KeyProblem::Invalid);
            }
            if let Some((_, owner)) = taken.iter().find(|(used, _)| *used == key) {
                return Err(KeyProblem::Taken(owner.clone()));
            }
            taken.push((key, title.clone()));
            Ok(Some(shortcut))
        })
        .collect()
}

/// Why a tool is off or has no shortcut, as the code `reader.slint` turns into a sentence:
/// 0 none, 1 needs a selection, 2 cannot replace here, 3 invalid shortcut, 4 reserved shortcut,
/// 5 shortcut used by another tool (`arg` names it), 6 the file is too large for tools, 7 the file
/// is not text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub enabled: bool,
    pub kind: i32,
    pub arg: String,
}

/// How the open file's raw content is held.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Loaded {
    Text,
    TooLarge,
    NotText,
}

/// What the reader can offer a tool right now.
#[derive(Clone, Copy)]
pub struct Context {
    pub editing: bool,
    pub selection: bool,
    pub loaded: Loaded,
    pub replaceable: bool,
}

/// Whether the tool can run in `context` and the note to show beside it.
pub fn note(
    input: Input,
    output: ToolOutput,
    context: Context,
    key: &Result<Option<Shortcut>, KeyProblem>,
) -> Note {
    let off = |kind| Note {
        enabled: false,
        kind,
        arg: String::new(),
    };
    if input == Input::Selection && !(context.editing && context.selection) {
        return off(1);
    }
    if input == Input::Document && !context.editing {
        match context.loaded {
            Loaded::Text => {}
            Loaded::TooLarge => return off(6),
            Loaded::NotText => return off(7),
        }
    }
    if output == ToolOutput::Replace && !context.editing && !context.replaceable {
        return off(2);
    }
    let (kind, arg) = match key {
        Ok(_) => (0, String::new()),
        Err(KeyProblem::Invalid) => (3, String::new()),
        Err(KeyProblem::Reserved) => (4, String::new()),
        Err(KeyProblem::Taken(owner)) => (5, owner.clone()),
    };
    Note {
        enabled: true,
        kind,
        arg,
    }
}

/// The bytes between the two ends of a selection, or `None` when it is empty or not on characters.
pub fn selection_range(text: &str, anchor: i32, cursor: i32) -> Option<Range<usize>> {
    let (a, b) = (usize::try_from(anchor).ok()?, usize::try_from(cursor).ok()?);
    let range = a.min(b)..a.max(b);
    (range.start < range.end
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end))
    .then_some(range)
}

/// `text` with `range` replaced by `replacement`.
pub fn splice(text: &str, range: Range<usize>, replacement: &str) -> String {
    format!(
        "{}{}{}",
        &text[..range.start],
        replacement,
        &text[range.end..]
    )
}

/// Why a run gave nothing to show, as the code `reader.slint` turns into a sentence (`run-reason`):
/// 0 `detail` as it is (the command's error or the system's), 1 exit code `detail`, 2 the command
/// stopped, 3 took longer than `detail` seconds, 4 wrote too much, 5 did not write UTF-8,
/// 6 cancelled, 7 the text changed while the tool ran, 8 no viewer for the type `detail`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason {
    pub kind: i32,
    pub detail: String,
}

impl Reason {
    pub fn new(kind: i32, detail: impl Into<String>) -> Reason {
        Reason {
            kind,
            detail: detail.into(),
        }
    }
}

/// How much of the command's error a banner shows.
const ERROR_CHARS: usize = 300;

/// The reason for a run that ended without output; a command's error is its first non-empty line.
pub fn describe(error: &RunError, seconds: u64) -> Reason {
    match error {
        RunError::Failed { stderr, code } => {
            match stderr.lines().find(|line| !line.trim().is_empty()) {
                Some(line) => Reason::new(0, shortened(line.trim(), ERROR_CHARS)),
                None => match code {
                    Some(code) => Reason::new(1, code.to_string()),
                    None => Reason::new(2, ""),
                },
            }
        }
        RunError::Spawn(error) => Reason::new(0, error.to_string()),
        RunError::Timeout => Reason::new(3, seconds.to_string()),
        RunError::TooMuchOutput => Reason::new(4, ""),
        RunError::InvalidUtf8 => Reason::new(5, ""),
        RunError::Cancelled => Reason::new(6, ""),
    }
}

/// `text` cut to `limit` characters, with an ellipsis when cut.
fn shortened(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(items: &[(&str, Option<&str>)]) -> Vec<Result<Option<Shortcut>, KeyProblem>> {
        let requested: Vec<(String, Option<String>)> = items
            .iter()
            .map(|(title, key)| (title.to_string(), key.map(str::to_string)))
            .collect();
        resolve(&requested)
    }

    #[test]
    fn a_valid_shortcut_is_kept() {
        let result = ask(&[("Formatar", Some("Ctrl+Shift+F"))]);
        assert_eq!(
            result[0].clone().unwrap().unwrap().canonical(),
            "ctrl+shift+f"
        );
    }

    #[test]
    fn no_shortcut_is_fine() {
        assert_eq!(ask(&[("A", None)]), [Ok(None)]);
    }

    #[test]
    fn the_readers_keys_are_reserved() {
        for key in [
            "Ctrl+E",
            "ctrl+s",
            "Ctrl+Shift+S",
            "shift+ctrl+e",
            "Ctrl+F",
            "Shift+F3",
            "Alt+Z",
        ] {
            assert_eq!(
                ask(&[("A", Some(key))]),
                [Err(KeyProblem::Reserved)],
                "{key}"
            );
        }
    }

    #[test]
    fn the_second_tool_asking_for_a_key_loses_it() {
        let result = ask(&[
            ("Formatar", Some("Ctrl+Shift+F")),
            ("Outra", Some("shift+ctrl+f")),
        ]);
        assert!(matches!(result[0], Ok(Some(_))));
        assert_eq!(result[1], Err(KeyProblem::Taken("Formatar".into())));
    }

    #[test]
    fn nonsense_is_invalid() {
        assert_eq!(ask(&[("A", Some("nada+x"))]), [Err(KeyProblem::Invalid)]);
        assert_eq!(ask(&[("A", Some("f"))]), [Err(KeyProblem::Invalid)]);
    }

    #[test]
    fn shortcuts_need_ctrl_or_alt() {
        for key in ["Shift+F5", "Win+K", "Shift+Win+K"] {
            assert_eq!(
                ask(&[("A", Some(key))]),
                [Err(KeyProblem::Invalid)],
                "{key}"
            );
        }
    }

    #[test]
    fn ctrl_alt_takes_only_letters_and_function_keys() {
        assert!(matches!(ask(&[("A", Some("Ctrl+Alt+K"))])[0], Ok(Some(_))));
        assert!(matches!(ask(&[("A", Some("Ctrl+Alt+F5"))])[0], Ok(Some(_))));
        assert!(matches!(ask(&[("A", Some("Alt+2"))])[0], Ok(Some(_))));
        assert!(matches!(ask(&[("A", Some("Ctrl+2"))])[0], Ok(Some(_))));
        for key in [
            "Ctrl+Alt+2",
            "Ctrl+Alt+Shift+Minus",
            "ctrl+alt+bracketleft",
            "Ctrl+Alt+Space",
        ] {
            assert_eq!(
                ask(&[("A", Some(key))]),
                [Err(KeyProblem::Invalid)],
                "{key}"
            );
        }
    }

    const EDITING: Context = Context {
        editing: true,
        selection: true,
        loaded: Loaded::Text,
        replaceable: true,
    };

    #[test]
    fn selection_tools_need_a_selection_in_edit_mode() {
        let ok = Ok(None);
        let idle = Context {
            editing: false,
            ..EDITING
        };
        let empty = Context {
            selection: false,
            ..EDITING
        };
        assert!(note(Input::Selection, ToolOutput::Show, EDITING, &ok).enabled);
        assert_eq!(note(Input::Selection, ToolOutput::Show, idle, &ok).kind, 1);
        assert_eq!(note(Input::Selection, ToolOutput::Show, empty, &ok).kind, 1);
    }

    #[test]
    fn replacing_needs_an_editable_document() {
        let ok = Ok(None);
        let locked = Context {
            editing: false,
            replaceable: false,
            ..EDITING
        };
        let note_ = note(Input::Document, ToolOutput::Replace, locked, &ok);
        assert!(!note_.enabled);
        assert_eq!(note_.kind, 2);
        assert!(note(Input::Document, ToolOutput::Show, locked, &ok).enabled);
        let large = Context {
            editing: false,
            loaded: Loaded::TooLarge,
            ..EDITING
        };
        assert_eq!(note(Input::Document, ToolOutput::Show, large, &ok).kind, 6);
        let binary = Context {
            loaded: Loaded::NotText,
            ..large
        };
        assert_eq!(note(Input::Document, ToolOutput::Show, binary, &ok).kind, 7);
    }

    #[test]
    fn a_bad_shortcut_leaves_the_tool_on_with_a_note() {
        let taken = Err(KeyProblem::Taken("Formatar".into()));
        let note_ = note(Input::Document, ToolOutput::Show, EDITING, &taken);
        assert!(note_.enabled);
        assert_eq!((note_.kind, note_.arg.as_str()), (5, "Formatar"));
        let reserved = Err(KeyProblem::Reserved);
        assert_eq!(
            note(Input::Document, ToolOutput::Show, EDITING, &reserved).kind,
            4
        );
        let invalid = Err(KeyProblem::Invalid);
        assert_eq!(
            note(Input::Document, ToolOutput::Show, EDITING, &invalid).kind,
            3
        );
    }

    #[test]
    fn selections_are_ordered_and_on_characters() {
        assert_eq!(selection_range("abcdef", 4, 1), Some(1..4));
        assert_eq!(selection_range("abcdef", 2, 2), None);
        assert_eq!(selection_range("abcdef", -1, 2), None);
        assert_eq!(selection_range("ação", 2, 4), None);
        assert_eq!(selection_range("ação", 0, 3), Some(0..3));
    }

    #[test]
    fn splice_replaces_the_range() {
        assert_eq!(splice("um dois tres", 3..7, "DOIS"), "um DOIS tres");
        assert_eq!(splice("abc", 0..3, ""), "");
    }

    #[test]
    fn run_errors_read_as_short_reasons() {
        let failed = |stderr: &str, code| RunError::Failed {
            code,
            stderr: stderr.into(),
        };
        assert_eq!(
            describe(&failed("boom", Some(1)), 10),
            Reason::new(0, "boom")
        );
        assert_eq!(
            describe(&failed("\n  \nfirst line\nsecond", Some(1)), 10),
            Reason::new(0, "first line")
        );
        assert_eq!(describe(&failed("", Some(3)), 10), Reason::new(1, "3"));
        assert_eq!(describe(&failed(" \n", None), 10), Reason::new(2, ""));
        assert_eq!(describe(&RunError::Timeout, 10), Reason::new(3, "10"));
        assert_eq!(describe(&RunError::TooMuchOutput, 10).kind, 4);
        assert_eq!(describe(&RunError::InvalidUtf8, 10).kind, 5);
    }

    #[test]
    fn a_long_error_line_is_cut() {
        let long = "é".repeat(1000);
        let reason = describe(
            &RunError::Failed {
                code: Some(1),
                stderr: long,
            },
            10,
        );
        assert_eq!(reason.detail.chars().count(), ERROR_CHARS + 1);
        assert!(reason.detail.ends_with('…'));
        assert_eq!(shortened("abc", 3), "abc");
    }
}
