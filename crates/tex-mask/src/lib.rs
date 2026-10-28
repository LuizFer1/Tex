mod template;

pub use template::{CharClass, MaskError, Part, Template};

use tex_index::fold;

/// Inputs larger than this are not worked on: the popup runs the commands on every edit.
pub const MAX_INPUT: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub name: String,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub name: String,
    pub template: Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    HasSpace,
    Duplicate,
}

impl NameError {
    pub fn code(self) -> i32 {
        match self {
            NameError::Empty => 1,
            NameError::HasSpace => 2,
            NameError::Duplicate => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmasked {
    pub text: String,
    pub count: usize,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    pub lines: usize,
}

pub fn defaults() -> Vec<Mask> {
    [
        (
            "cnpj",
            r"([0-9A-Z]{2})\.([0-9A-Z]{3})\.([0-9A-Z]{3})/([0-9A-Z]{4})-(\d{2})",
        ),
        ("cpf", r"(\d{3})\.(\d{3})\.(\d{3})-(\d{2})"),
        ("cep", r"(\d{5})-(\d{3})"),
        ("celular", r"\((\d{2})\) (\d{5})-(\d{4})"),
        ("fixo", r"\((\d{2})\) (\d{4})-(\d{4})"),
    ]
    .into_iter()
    .map(|(name, pattern)| Mask {
        name: name.to_owned(),
        pattern: pattern.to_owned(),
    })
    .collect()
}

/// A name is one word, and unique once accents and case are folded away.
pub fn validate_name(name: &str, others: &[&str]) -> Result<(), NameError> {
    if name.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().any(char::is_whitespace) {
        return Err(NameError::HasSpace);
    }
    let folded = fold(name);
    if others.iter().any(|other| fold(other) == folded) {
        return Err(NameError::Duplicate);
    }
    Ok(())
}

/// Keeps the masks that can be used, in order. A name already taken by an earlier mask, a name
/// that is not one word, or a pattern that does not parse, is left out.
pub fn compile(masks: &[Mask]) -> Vec<Compiled> {
    let mut compiled: Vec<Compiled> = Vec::new();
    for mask in masks {
        let taken: Vec<&str> = compiled.iter().map(|mask| mask.name.as_str()).collect();
        if validate_name(&mask.name, &taken).is_err() {
            continue;
        }
        if let Ok(template) = Template::parse(&mask.pattern) {
            compiled.push(Compiled {
                name: mask.name.clone(),
                template,
            });
        }
    }
    compiled
}

pub fn find(masks: &[Compiled], name: &str) -> Option<usize> {
    let name = fold(name);
    masks.iter().position(|mask| fold(&mask.name) == name)
}

/// Indexes of the masks whose name contains `term`, at most `limit`.
pub fn filter(masks: &[Compiled], term: &str, limit: usize) -> Vec<usize> {
    let term = fold(term);
    masks
        .iter()
        .enumerate()
        .filter(|(_, mask)| fold(&mask.name).contains(&term))
        .map(|(index, _)| index)
        .take(limit)
        .collect()
}

/// Indexes of the masks every non-empty line of `text` fits, at most `limit`.
pub fn suggest(masks: &[Compiled], text: &str, limit: usize) -> Vec<usize> {
    masks
        .iter()
        .enumerate()
        .filter(|(_, mask)| fits(&mask.template, text))
        .map(|(index, _)| index)
        .take(limit)
        .collect()
}

fn fits(template: &Template, text: &str) -> bool {
    if text.len() > MAX_INPUT {
        return false;
    }
    let mut any = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        any = true;
        if template.format(line).is_none() {
            return false;
        }
    }
    any
}

/// Takes every masked number of any mask out of `text`. Where several masks match at the same
/// point the longest wins, then the first of the list. A match touching a letter or digit
/// does not count.
pub fn unmask(masks: &[Compiled], text: &str) -> Option<Unmasked> {
    if text.len() > MAX_INPUT {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut found: Vec<(usize, usize, usize, String)> = Vec::new();
    for start in 0..chars.len() {
        if start > 0 && chars[start - 1].is_alphanumeric() {
            continue;
        }
        for (index, mask) in masks.iter().enumerate() {
            if let Some((end, payload)) = mask.template.match_at(&chars, start) {
                if chars.get(end).is_some_and(|c| c.is_alphanumeric()) {
                    continue;
                }
                found.push((start, end, index, payload));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    let mut out = String::new();
    let mut cursor = 0;
    let mut count = 0;
    let mut names: Vec<String> = Vec::new();
    for (start, end, index, payload) in found {
        if start < cursor {
            continue;
        }
        out.extend(chars[cursor..start].iter());
        out.push_str(&payload);
        cursor = end;
        count += 1;
        let name = &masks[index].name;
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    if count == 0 {
        return None;
    }
    out.extend(chars[cursor..].iter());
    Some(Unmasked {
        text: out,
        count,
        names,
    })
}

/// Puts the mask on each line of `text` that fits it. `None` when no line changes.
pub fn apply(template: &Template, text: &str) -> Option<Formatted> {
    if text.len() > MAX_INPUT {
        return None;
    }
    let mut lines = 0;
    let out: Vec<String> = text
        .split('\n')
        .map(|line| {
            let (body, ending) = match line.strip_suffix('\r') {
                Some(body) => (body, "\r"),
                None => (line, ""),
            };
            match template.format(body) {
                Some(formatted) if formatted != body => {
                    lines += 1;
                    format!("{formatted}{ending}")
                }
                _ => line.to_owned(),
            }
        })
        .collect();
    (lines > 0).then(|| Formatted {
        text: out.join("\n"),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults_compiled() -> Vec<Compiled> {
        compile(&defaults())
    }

    #[test]
    fn every_default_compiles() {
        assert_eq!(defaults_compiled().len(), defaults().len());
        assert_eq!(defaults().len(), 5);
    }

    #[test]
    fn unmask_round_trips_every_default() {
        let masks = defaults_compiled();
        for (name, raw, masked) in [
            ("cnpj", "12345678000195", "12.345.678/0001-95"),
            ("cnpj", "12ABC345000195", "12.ABC.345/0001-95"),
            ("cpf", "12345678909", "123.456.789-09"),
            ("cep", "01310100", "01310-100"),
            ("celular", "11987654321", "(11) 98765-4321"),
            ("fixo", "1134567890", "(11) 3456-7890"),
        ] {
            let index = find(&masks, name).unwrap();
            let formatted = apply(&masks[index].template, raw).unwrap();
            assert_eq!(formatted.text, masked, "{name}");
            let cleaned = unmask(&masks, masked).unwrap();
            assert_eq!(cleaned.text, raw, "{name}");
        }
    }

    #[test]
    fn unmask_replaces_every_occurrence_and_keeps_the_rest() {
        let masks = defaults_compiled();
        let result = unmask(
            &masks,
            "Empresa X — 12.345.678/0001-95\r\nY: 98.765.432/0001-10 e CEP 01310-100",
        )
        .unwrap();
        assert_eq!(
            result.text,
            "Empresa X — 12345678000195\r\nY: 98765432000110 e CEP 01310100"
        );
        assert_eq!(result.count, 3);
        assert_eq!(result.names, vec!["cnpj".to_owned(), "cep".to_owned()]);
    }

    #[test]
    fn unmask_ignores_a_match_glued_to_letters_or_digits() {
        let masks = defaults_compiled();
        assert_eq!(unmask(&masks, "112.345.678/0001-95"), None);
        assert_eq!(unmask(&masks, "12.345.678/0001-955"), None);
        assert_eq!(unmask(&masks, "x12.345.678/0001-95"), None);
    }

    #[test]
    fn a_rejected_start_does_not_hide_a_valid_one_later() {
        let masks = defaults_compiled();
        let result = unmask(&masks, "1 12.345.678/0001-95").unwrap();
        assert_eq!(result.text, "1 12345678000195");
    }

    #[test]
    fn unmask_prefers_the_longest_match_then_the_first_mask() {
        let both = compile(&[
            Mask {
                name: "curto".into(),
                pattern: r"(\d{2})-(\d{2})".into(),
            },
            Mask {
                name: "longo".into(),
                pattern: r"(\d{2})-(\d{2})-(\d{2})".into(),
            },
            Mask {
                name: "outro".into(),
                pattern: r"(\d{2})/(\d{2})/(\d{2})".into(),
            },
        ]);
        let result = unmask(&both, "11-22-33").unwrap();
        assert_eq!(result.text, "112233");
        assert_eq!(result.names, vec!["longo".to_owned()]);
    }

    #[test]
    fn unmask_without_any_match_or_above_the_limit_is_none() {
        let masks = defaults_compiled();
        assert_eq!(unmask(&masks, "nada para limpar 123"), None);
        assert_eq!(unmask(&masks, &"a".repeat(MAX_INPUT + 1)), None);
    }

    #[test]
    fn apply_formats_each_line_and_leaves_the_others() {
        let masks = defaults_compiled();
        let cnpj = &masks[find(&masks, "cnpj").unwrap()].template;
        let result = apply(cnpj, "12345678000195\r\nnao e cnpj\n98765432000110").unwrap();
        assert_eq!(
            result.text,
            "12.345.678/0001-95\r\nnao e cnpj\n98.765.432/0001-10"
        );
        assert_eq!(result.lines, 2);
    }

    #[test]
    fn apply_with_nothing_to_change_is_none() {
        let masks = defaults_compiled();
        let cnpj = &masks[find(&masks, "cnpj").unwrap()].template;
        assert_eq!(apply(cnpj, "12.345.678/0001-95"), None);
        assert_eq!(apply(cnpj, "abc"), None);
        assert_eq!(apply(cnpj, ""), None);
    }

    #[test]
    fn filter_matches_names_without_accent_or_case_and_limits() {
        let masks = defaults_compiled();
        assert_eq!(filter(&masks, "CN", 5), vec![0]);
        assert_eq!(filter(&masks, "", 3).len(), 3);
        assert_eq!(filter(&masks, "zzz", 5), Vec::<usize>::new());
        assert_eq!(find(&masks, "CPF"), Some(1));
    }

    #[test]
    fn suggest_lists_the_masks_that_fit_the_text() {
        let masks = defaults_compiled();
        let names = |text: &str| -> Vec<String> {
            suggest(&masks, text, 5)
                .into_iter()
                .map(|i| masks[i].name.clone())
                .collect()
        };
        assert_eq!(names("12345678000195"), vec!["cnpj"]);
        assert_eq!(names("12345678909"), vec!["cpf", "celular"]);
        assert_eq!(names("01310100"), vec!["cep"]);
        assert_eq!(names("12345678000195\n98765432000110"), vec!["cnpj"]);
        assert_eq!(names("12345678000195\nabc"), Vec::<String>::new());
        assert_eq!(names(""), Vec::<String>::new());
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(validate_name("cnpj", &[]), Ok(()));
        assert_eq!(validate_name("", &[]), Err(NameError::Empty));
        assert_eq!(validate_name("meu cnpj", &[]), Err(NameError::HasSpace));
        assert_eq!(validate_name("CNPJ", &["cnpj"]), Err(NameError::Duplicate));
        assert_eq!(validate_name("Ação", &["acao"]), Err(NameError::Duplicate));
    }

    #[test]
    fn compile_skips_what_is_unusable() {
        let compiled = compile(&[
            Mask {
                name: "ok".into(),
                pattern: r"(\d{2})-(\d{2})".into(),
            },
            Mask {
                name: "ruim".into(),
                pattern: r"(\d+)-(\d{2})".into(),
            },
            Mask {
                name: "com espaco".into(),
                pattern: r"(\d{2})-(\d{2})".into(),
            },
        ]);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].name, "ok");
    }
}
