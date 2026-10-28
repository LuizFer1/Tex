use tex_mask::Compiled;

use crate::results::{Selection, step_with_answer};

pub const MAX_SUGGESTIONS: usize = 5;
pub const MAX_CLIPS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Clean,
    Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub keyword: String,
    pub kind: Kind,
    pub rest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub output: String,
    pub mask: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub suggestions: Vec<usize>,
    pub typed: String,
    pub input: String,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Masks,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(Section),
    Suggestion(usize),
    Clip(usize),
}

/// The command a query starts with: its word, followed by a space.
pub fn parse(query: &str) -> Option<Command> {
    let (word, rest) = query.trim_start().split_once(char::is_whitespace)?;
    let kind = match tex_index::fold(word).as_str() {
        "limpar" | "clean" => Kind::Clean,
        "formatar" | "maskup" => Kind::Format,
        _ => return None,
    };
    Some(Command {
        keyword: word.to_owned(),
        kind,
        rest: rest.trim_start().to_owned(),
    })
}

fn split_term(rest: &str) -> (&str, &str) {
    rest.split_once(char::is_whitespace)
        .map_or((rest, ""), |(term, after)| (term, after.trim_start()))
}

/// What the panel shows for `command`: the text it works on (typed, or the clip at `chosen`),
/// the mask suggestions and the outcome when there is one.
pub fn view(command: &Command, masks: &[Compiled], clips: &[String], chosen: usize) -> View {
    let clip = clips.get(chosen).cloned().unwrap_or_default();
    let pick = |typed: &str| {
        if typed.is_empty() {
            clip.clone()
        } else {
            typed.to_owned()
        }
    };
    if command.kind == Kind::Clean {
        let input = pick(&command.rest);
        let outcome = tex_mask::unmask(masks, &input).map(|result| Outcome {
            output: result.text,
            mask: result.names.join(", "),
            count: result.count,
        });
        return View {
            suggestions: Vec::new(),
            typed: command.rest.clone(),
            input,
            outcome,
        };
    }
    let (term, after) = split_term(&command.rest);
    if let Some(index) = tex_mask::find(masks, term) {
        let input = pick(after);
        let outcome = tex_mask::apply(&masks[index].template, &input).map(|result| Outcome {
            output: result.text,
            mask: masks[index].name.clone(),
            count: result.lines,
        });
        return View {
            suggestions: tex_mask::filter(masks, term, MAX_SUGGESTIONS),
            typed: after.to_owned(),
            input,
            outcome,
        };
    }
    let by_name = if term.is_empty() {
        Vec::new()
    } else {
        tex_mask::filter(masks, term, MAX_SUGGESTIONS)
    };
    if !by_name.is_empty() {
        return View {
            suggestions: by_name,
            typed: after.to_owned(),
            input: pick(after),
            outcome: None,
        };
    }
    let input = pick(&command.rest);
    let mut suggestions = tex_mask::suggest(masks, &input, MAX_SUGGESTIONS);
    if command.rest.is_empty() {
        for index in 0..masks.len() {
            if suggestions.len() >= MAX_SUGGESTIONS {
                break;
            }
            if !suggestions.contains(&index) {
                suggestions.push(index);
            }
        }
    }
    View {
        suggestions,
        typed: command.rest.clone(),
        input,
        outcome: None,
    }
}

/// The query after choosing the suggested mask `name`.
pub fn complete(command: &Command, view: &View, name: &str) -> String {
    format!("{} {} {}", command.keyword, name, view.typed)
}

pub fn rows(view: &View, clips: &[String]) -> Vec<Row> {
    let mut rows = Vec::new();
    if !view.suggestions.is_empty() {
        rows.push(Row::Header(Section::Masks));
        rows.extend(view.suggestions.iter().map(|&index| Row::Suggestion(index)));
    }
    if !clips.is_empty() {
        rows.push(Row::Header(Section::History));
        rows.extend((0..clips.len()).map(Row::Clip));
    }
    rows
}

/// The first non-empty line, cut at `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Choose(usize),
    Nothing,
}

/// What Enter does on `selection`: copy the result (the card, or a clip that has one), complete
/// the query with a suggestion (its mask index), or nothing.
pub fn action(selection: Option<Selection>, rows: &[Row], has_result: bool) -> Action {
    match selection {
        Some(Selection::Answer) if has_result => Action::Copy,
        Some(Selection::Row(index)) => match rows.get(index) {
            Some(Row::Suggestion(mask)) => Action::Choose(*mask),
            Some(Row::Clip(_)) if has_result => Action::Copy,
            _ => Action::Nothing,
        },
        _ => Action::Nothing,
    }
}

/// The selection a fresh panel starts with: the card when there is one, else the first line that
/// can be picked.
pub fn initial_selection(has_card: bool, rows: &[Row]) -> Option<Selection> {
    let is_item: Vec<bool> = rows
        .iter()
        .map(|row| !matches!(row, Row::Header(_)))
        .collect();
    step_with_answer(has_card, &is_item, None, 1)
}

/// The selection after the panel was rebuilt from `previous` to `rows`: the same line (matched by
/// value, so an added clip does not move it), or the initial one when that line is gone.
pub fn settle(
    selection: Option<Selection>,
    previous: &[Row],
    rows: &[Row],
    has_card: bool,
) -> Option<Selection> {
    let kept = match selection {
        Some(Selection::Answer) if has_card => Some(Selection::Answer),
        Some(Selection::Row(index)) => previous
            .get(index)
            .filter(|row| !matches!(row, Row::Header(_)))
            .and_then(|row| rows.iter().position(|next| next == row))
            .map(Selection::Row),
        _ => None,
    };
    kept.or_else(|| initial_selection(has_card, rows))
}

/// The query the history shortcut opens the popup with.
#[allow(dead_code)]
pub fn history_query(masks: &[Compiled], text: &str, english: bool) -> String {
    let clean = tex_mask::unmask(masks, text).is_some();
    match (clean, english) {
        (true, false) => "limpar ",
        (true, true) => "clean ",
        (false, false) => "formatar ",
        (false, true) => "maskup ",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks() -> Vec<Compiled> {
        tex_mask::compile(&tex_mask::defaults())
    }

    fn names(masks: &[Compiled], view: &View) -> Vec<String> {
        view.suggestions
            .iter()
            .map(|&i| masks[i].name.clone())
            .collect()
    }

    #[test]
    fn reads_the_command_words_in_both_languages() {
        assert_eq!(parse("limpar 123").unwrap().kind, Kind::Clean);
        assert_eq!(parse("CLEAN 123").unwrap().kind, Kind::Clean);
        assert_eq!(parse("formatar cnpj 1").unwrap().kind, Kind::Format);
        assert_eq!(parse("maskup").map(|c| c.kind), None);
        assert_eq!(parse("limpar").map(|c| c.kind), None);
        assert_eq!(parse("limpar ").unwrap().rest, "");
        assert_eq!(parse("limparx 1"), None);
        assert_eq!(parse("abrir limpar"), None);
    }

    #[test]
    fn clean_works_on_the_typed_text_or_the_chosen_clip() {
        let masks = masks();
        let clips = vec![
            "12.345.678/0001-95".to_owned(),
            "98.765.432/0001-10".to_owned(),
        ];
        let typed = view(&parse("limpar 123.456.789-09").unwrap(), &masks, &clips, 0);
        assert_eq!(typed.outcome.unwrap().output, "12345678909");
        let clip = view(&parse("limpar ").unwrap(), &masks, &clips, 1);
        assert_eq!(clip.input, "98.765.432/0001-10");
        assert_eq!(clip.outcome.unwrap().output, "98765432000110");
        let none = view(&parse("limpar nada").unwrap(), &masks, &clips, 0);
        assert_eq!(none.outcome, None);
    }

    #[test]
    fn format_with_a_known_name_applies_it() {
        let masks = masks();
        let view = view(
            &parse("formatar CNPJ 12345678000195").unwrap(),
            &masks,
            &[],
            0,
        );
        let outcome = view.outcome.unwrap();
        assert_eq!(outcome.output, "12.345.678/0001-95");
        assert_eq!(outcome.mask, "cnpj");
        assert!(!view.suggestions.is_empty());
    }

    #[test]
    fn format_with_a_partial_name_filters_the_masks() {
        let masks = masks();
        let clips = vec!["12345678000195".to_owned()];
        let result = view(&parse("formatar c").unwrap(), &masks, &clips, 0);
        assert_eq!(
            names(&masks, &result),
            vec!["cnpj", "cpf", "cep", "celular"]
        );
        assert_eq!(result.outcome, None);
        let result = view(&parse("formatar cn").unwrap(), &masks, &clips, 0);
        assert_eq!(names(&masks, &result), vec!["cnpj"]);
    }

    #[test]
    fn format_with_text_that_is_not_a_name_suggests_by_shape() {
        let masks = masks();
        let result = view(&parse("formatar 12345678909").unwrap(), &masks, &[], 0);
        assert_eq!(names(&masks, &result), vec!["cpf", "celular"]);
        assert_eq!(result.typed, "12345678909");
        assert_eq!(result.outcome, None);
    }

    #[test]
    fn format_with_nothing_typed_puts_the_masks_that_fit_the_clip_first() {
        let masks = masks();
        let clips = vec!["01310100".to_owned()];
        let result = view(&parse("formatar ").unwrap(), &masks, &clips, 0);
        assert_eq!(
            names(&masks, &result),
            vec!["cep", "cnpj", "cpf", "celular", "fixo"]
        );
    }

    #[test]
    fn suggestions_are_capped() {
        let many: Vec<tex_mask::Mask> = (0..9)
            .map(|i| tex_mask::Mask {
                name: format!("m{i}"),
                pattern: r"(\d)-(\d)".into(),
            })
            .collect();
        let masks = tex_mask::compile(&many);
        let result = view(&parse("formatar m").unwrap(), &masks, &[], 0);
        assert_eq!(result.suggestions.len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn completing_a_suggestion_rewrites_the_query() {
        let masks = masks();
        let command = parse("maskup 12345678000195").unwrap();
        let result = view(&command, &masks, &[], 0);
        assert_eq!(
            complete(&command, &result, "cnpj"),
            "maskup cnpj 12345678000195"
        );
        let command = parse("formatar cn").unwrap();
        let result = view(&command, &masks, &[], 0);
        assert_eq!(complete(&command, &result, "cnpj"), "formatar cnpj ");
    }

    #[test]
    fn rows_have_a_header_per_non_empty_section() {
        let masks = masks();
        let clips = vec!["a".to_owned(), "b".to_owned()];
        let result = view(&parse("formatar cn").unwrap(), &masks, &clips, 0);
        assert_eq!(
            rows(&result, &clips),
            vec![
                Row::Header(Section::Masks),
                Row::Suggestion(0),
                Row::Header(Section::History),
                Row::Clip(0),
                Row::Clip(1),
            ]
        );
        let result = view(&parse("limpar x").unwrap(), &masks, &[], 0);
        assert!(rows(&result, &[]).is_empty());
    }

    #[test]
    fn a_preview_is_the_first_non_empty_line_cut_at_the_limit() {
        assert_eq!(preview("\n  \nabc\ndef", 10), "abc");
        assert_eq!(preview("abcdefghij", 5), "abcde…");
        assert_eq!(preview("ação ação", 4), "ação…");
        assert_eq!(preview("", 5), "");
    }

    #[test]
    fn enter_copies_the_card_or_a_clip_with_a_result_and_completes_a_suggestion() {
        let rows = [
            Row::Header(Section::Masks),
            Row::Suggestion(2),
            Row::Header(Section::History),
            Row::Clip(0),
            Row::Clip(1),
        ];
        assert_eq!(action(Some(Selection::Answer), &rows, true), Action::Copy);
        assert_eq!(
            action(Some(Selection::Answer), &rows, false),
            Action::Nothing
        );
        assert_eq!(
            action(Some(Selection::Row(1)), &rows, true),
            Action::Choose(2)
        );
        assert_eq!(action(Some(Selection::Row(3)), &rows, true), Action::Copy);
        assert_eq!(
            action(Some(Selection::Row(4)), &rows, false),
            Action::Nothing
        );
        assert_eq!(
            action(Some(Selection::Row(0)), &rows, true),
            Action::Nothing
        );
        assert_eq!(action(None, &rows, true), Action::Nothing);
    }

    #[test]
    fn a_fresh_panel_starts_on_the_card_or_the_first_line_to_pick() {
        let rows = [
            Row::Header(Section::Masks),
            Row::Suggestion(0),
            Row::Header(Section::History),
            Row::Clip(0),
        ];
        assert_eq!(initial_selection(true, &rows), Some(Selection::Answer));
        assert_eq!(initial_selection(false, &rows), Some(Selection::Row(1)));
        assert_eq!(
            initial_selection(false, &[Row::Header(Section::History), Row::Clip(0)]),
            Some(Selection::Row(1))
        );
        assert_eq!(
            initial_selection(false, &[Row::Header(Section::Masks)]),
            None
        );
    }

    #[test]
    fn a_rebuilt_panel_keeps_the_same_line_by_value() {
        let before = [Row::Header(Section::History), Row::Clip(0), Row::Clip(1)];
        let after = [
            Row::Header(Section::Masks),
            Row::Suggestion(0),
            Row::Header(Section::History),
            Row::Clip(0),
            Row::Clip(1),
        ];
        assert_eq!(
            settle(Some(Selection::Row(2)), &before, &after, false),
            Some(Selection::Row(4))
        );
        assert_eq!(
            settle(Some(Selection::Row(1)), &before, &after, false),
            Some(Selection::Row(3))
        );
        assert_eq!(
            settle(Some(Selection::Row(0)), &before, &after, false),
            Some(Selection::Row(1))
        );
        assert_eq!(
            settle(Some(Selection::Answer), &before, &after, false),
            Some(Selection::Row(1))
        );
        assert_eq!(
            settle(Some(Selection::Answer), &before, &after, true),
            Some(Selection::Answer)
        );
        assert_eq!(
            settle(
                Some(Selection::Row(2)),
                &before,
                &[Row::Header(Section::History)],
                false
            ),
            None
        );
        assert_eq!(settle(None, &[], &after, true), Some(Selection::Answer));
    }

    #[test]
    fn the_history_shortcut_picks_the_command_that_does_something() {
        let masks = masks();
        assert_eq!(
            history_query(&masks, "12.345.678/0001-95", false),
            "limpar "
        );
        assert_eq!(history_query(&masks, "12345678000195", false), "formatar ");
        assert_eq!(history_query(&masks, "12.345.678/0001-95", true), "clean ");
        assert_eq!(history_query(&masks, "12345678000195", true), "maskup ");
    }
}
