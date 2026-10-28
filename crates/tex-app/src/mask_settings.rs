use tex_mask::{MaskError, Template, validate_name};

use crate::settings::MaskEntry;

/// Added to a `MaskError` code, so pattern codes (11 to 15) never equal name codes (1 to 3).
const PATTERN_BASE: i32 = 10;

/// The code of the first problem, ignoring the entry at `index` (`None` for a new mask).
pub fn check(entries: &[MaskEntry], index: Option<usize>, name: &str, pattern: &str) -> i32 {
    let others: Vec<&str> = entries
        .iter()
        .enumerate()
        .filter(|(position, _)| Some(*position) != index)
        .map(|(_, entry)| entry.name.as_str())
        .collect();
    problem(name, pattern, &others)
}

/// Codes of the saved entries, a name counting as taken only by an earlier accepted entry, as
/// `tex_mask::compile` does.
pub fn errors(entries: &[MaskEntry]) -> Vec<i32> {
    let mut taken: Vec<&str> = Vec::new();
    entries
        .iter()
        .map(|entry| {
            let code = problem(&entry.name, &entry.pattern, &taken);
            if code == 0 {
                taken.push(entry.name.as_str());
            }
            code
        })
        .collect()
}

/// The row being edited (`-1` for none) once the row `deleted` is gone: it follows its mask, and
/// stops when that mask is the deleted one.
pub fn editing_after_delete(editing: i32, deleted: usize) -> i32 {
    let deleted = i32::try_from(deleted).unwrap_or(i32::MAX);
    if editing < 0 || editing < deleted {
        editing
    } else if editing == deleted {
        -1
    } else {
        editing - 1
    }
}

fn problem(name: &str, pattern: &str, others: &[&str]) -> i32 {
    if let Err(error) = validate_name(name, others) {
        return error.code();
    }
    if pattern.is_empty() {
        return PATTERN_BASE + MaskError::NoGroups.code();
    }
    match Template::parse(pattern) {
        Ok(_) => 0,
        Err(error) => PATTERN_BASE + error.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CNPJ: &str = r"([0-9A-Z]{2})\.([0-9A-Z]{3})\.([0-9A-Z]{3})/([0-9A-Z]{4})-(\d{2})";
    const CPF: &str = r"(\d{3})\.(\d{3})\.(\d{3})-(\d{2})";
    const PAIR: &str = r"(\d{2})-(\d{2})";

    fn entry(name: &str, pattern: &str) -> MaskEntry {
        MaskEntry {
            name: name.into(),
            pattern: pattern.into(),
        }
    }

    #[test]
    fn a_good_mask_can_be_saved() {
        assert_eq!(check(&[], None, "meu", PAIR), 0);
    }

    #[test]
    fn names_are_checked_before_patterns() {
        assert_eq!(check(&[], None, "", PAIR), 1);
        assert_eq!(check(&[], None, "   ", "("), 1);
        assert_eq!(check(&[], None, "meu cnpj", PAIR), 2);
    }

    #[test]
    fn a_name_taken_by_another_mask_is_refused_in_any_case_or_accent() {
        let entries = [entry("cnpj", CNPJ), entry("acao", PAIR)];
        assert_eq!(check(&entries, None, "CNPJ", PAIR), 3);
        assert_eq!(check(&entries, None, "Ação", PAIR), 3);
        assert_eq!(check(&entries, None, "outro", PAIR), 0);
    }

    #[test]
    fn an_entry_does_not_clash_with_its_own_name() {
        let entries = [entry("cnpj", CNPJ), entry("cpf", CPF)];
        assert_eq!(check(&entries, Some(0), "cnpj", CNPJ), 0);
        assert_eq!(check(&entries, Some(0), "CPF", PAIR), 3);
        assert_eq!(check(&entries, Some(1), "cnpj", PAIR), 3);
    }

    #[test]
    fn patterns_report_the_mask_error_code() {
        assert_eq!(check(&[], None, "x", "("), 11);
        assert_eq!(check(&[], None, "x", r"\d+"), 12);
        assert_eq!(check(&[], None, "x", "abc"), 13);
        assert_eq!(check(&[], None, "x", r"(\d{3})(\d{3})"), 14);
        assert_eq!(check(&[], None, "x", r"(\d{99})-(\d)"), 15);
    }

    #[test]
    fn an_empty_pattern_has_no_group() {
        assert_eq!(check(&[], None, "x", ""), 13);
    }

    #[test]
    fn the_row_being_edited_follows_its_mask_when_another_is_deleted() {
        assert_eq!(editing_after_delete(2, 0), 1);
        assert_eq!(editing_after_delete(2, 1), 1);
        assert_eq!(editing_after_delete(2, 2), -1);
        assert_eq!(editing_after_delete(0, 2), 0);
        assert_eq!(editing_after_delete(-1, 0), -1);
    }

    #[test]
    fn saved_rows_show_exactly_the_masks_that_compile_leaves_out() {
        let entries = vec![
            entry("cnpj", CNPJ),
            entry("CNPJ", PAIR),
            entry("ruim", r"(\d+)-(\d{2})"),
            entry("ok", PAIR),
            entry("ação", PAIR),
            entry("acao", PAIR),
            entry("futuro", "("),
            entry("futuro", PAIR),
        ];
        let codes = errors(&entries);
        assert_eq!(codes, vec![0, 3, 12, 0, 0, 3, 11, 0]);
        let masks: Vec<tex_mask::Mask> = entries
            .iter()
            .map(|entry| tex_mask::Mask {
                name: entry.name.clone(),
                pattern: entry.pattern.clone(),
            })
            .collect();
        let kept: Vec<String> = tex_mask::compile(&masks)
            .into_iter()
            .map(|mask| mask.name)
            .collect();
        let accepted: Vec<String> = entries
            .iter()
            .zip(&codes)
            .filter(|(_, code)| **code == 0)
            .map(|(entry, _)| entry.name.clone())
            .collect();
        assert_eq!(kept, accepted);
    }
}
