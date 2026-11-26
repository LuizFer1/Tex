use tex_mask::{GenError, GenSource, Pattern, validate_name};

use crate::settings::GeneratorEntry;

/// Added to a `GenError` code, so pattern codes (11 to 16) never equal name codes (1 to 3).
const PATTERN_BASE: i32 = 10;

/// The code of the first problem, ignoring the entry at `index` (`None` for a new generator). A
/// built-in generator only has its name checked, its pattern is not used.
pub fn check(entries: &[GeneratorEntry], index: Option<usize>, name: &str, pattern: &str) -> i32 {
    let others: Vec<&str> = entries
        .iter()
        .enumerate()
        .filter(|(position, _)| Some(*position) != index)
        .map(|(_, entry)| entry.name.as_str())
        .collect();
    if let Err(error) = validate_name(name, &others) {
        return error.code();
    }
    let builtin = index
        .and_then(|position| entries.get(position))
        .is_some_and(|entry| matches!(entry.source, GenSource::Builtin(_)));
    if builtin {
        return 0;
    }
    pattern_problem(pattern)
}

/// Codes of the saved entries, a name counting as taken only by an earlier accepted entry, as
/// `tex_mask::compile_generators` does.
pub fn errors(entries: &[GeneratorEntry]) -> Vec<i32> {
    let mut taken: Vec<&str> = Vec::new();
    entries
        .iter()
        .map(|entry| {
            let code = problem(&entry.name, &entry.source, &taken);
            if code == 0 {
                taken.push(entry.name.as_str());
            }
            code
        })
        .collect()
}

fn problem(name: &str, source: &GenSource, others: &[&str]) -> i32 {
    if let Err(error) = validate_name(name, others) {
        return error.code();
    }
    match source {
        GenSource::Builtin(_) => 0,
        GenSource::Pattern(pattern) => pattern_problem(pattern),
    }
}

fn pattern_problem(pattern: &str) -> i32 {
    if pattern.is_empty() {
        return PATTERN_BASE + GenError::NoRandom.code();
    }
    match Pattern::parse(pattern) {
        Ok(_) => 0,
        Err(error) => PATTERN_BASE + error.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_mask::{Builtin, GenSpec, compile_generators};

    fn entry(name: &str, source: GenSource) -> GeneratorEntry {
        GeneratorEntry {
            name: name.into(),
            source,
        }
    }

    fn pattern(name: &str, text: &str) -> GeneratorEntry {
        entry(name, GenSource::Pattern(text.into()))
    }

    #[test]
    fn a_good_generator_can_be_saved() {
        assert_eq!(check(&[], None, "meu", r"\d{3}"), 0);
    }

    #[test]
    fn names_are_checked_first() {
        assert_eq!(check(&[], None, "", r"\d{3}"), 1);
        assert_eq!(check(&[], None, "   ", "("), 1);
        assert_eq!(check(&[], None, "meu codigo", r"\d{3}"), 2);
    }

    #[test]
    fn a_name_taken_by_another_generator_is_refused_in_any_case_or_accent() {
        let entries = [
            entry("cpf", GenSource::Builtin(Builtin::Cpf)),
            pattern("codigo", r"\d{3}"),
        ];
        assert_eq!(check(&entries, None, "CPF", r"\d{3}"), 3);
        assert_eq!(check(&entries, None, "Código", r"\d{3}"), 3);
        assert_eq!(check(&entries, None, "outro", r"\d{3}"), 0);
    }

    #[test]
    fn a_pattern_reports_the_generator_error_code() {
        assert_eq!(check(&[], None, "x", "("), 11);
        assert_eq!(check(&[], None, "x", r"\d+"), 12);
        assert_eq!(check(&[], None, "x", "abc"), 13);
        assert_eq!(check(&[], None, "x", r"\d{300}"), 15);
        assert_eq!(check(&[], None, "x", r"[\x00-\x1F]"), 16);
    }

    #[test]
    fn an_empty_pattern_has_nothing_random() {
        assert_eq!(check(&[], None, "x", ""), 13);
    }

    #[test]
    fn a_builtin_only_has_its_name_checked() {
        let entries = [
            entry("cpf", GenSource::Builtin(Builtin::Cpf)),
            entry("cnpj", GenSource::Builtin(Builtin::Cnpj)),
        ];
        assert_eq!(check(&entries, Some(0), "documento", ""), 0);
        assert_eq!(check(&entries, Some(0), "cpf", "("), 0);
        assert_eq!(check(&entries, Some(0), "cnpj", ""), 3);
        assert_eq!(check(&entries, Some(0), "", ""), 1);
    }

    #[test]
    fn a_pattern_entry_checks_its_pattern_when_edited() {
        let entries = [pattern("codigo", r"\d{3}")];
        assert_eq!(check(&entries, Some(0), "codigo", "("), 11);
        assert_eq!(check(&entries, Some(0), "codigo", r"\d{4}"), 0);
    }

    #[test]
    fn saved_rows_show_exactly_the_generators_that_compile_keeps() {
        let entries = vec![
            entry("cpf", GenSource::Builtin(Builtin::Cpf)),
            entry("CPF", GenSource::Builtin(Builtin::Cpf)),
            pattern("ruim", r"\d+"),
            pattern("ok", r"(\d{2})-(\d{2})"),
            pattern("ação", r"\d{3}"),
            pattern("acao", r"\d{3}"),
            pattern("futuro", "("),
            pattern("futuro", r"\d{3}"),
            entry("senha", GenSource::Builtin(Builtin::Password)),
            entry("mau nome", GenSource::Builtin(Builtin::Cnpj)),
        ];
        let codes = errors(&entries);
        assert_eq!(codes, vec![0, 3, 12, 0, 0, 3, 11, 0, 0, 2]);
        let specs: Vec<GenSpec> = entries
            .iter()
            .map(|entry| GenSpec {
                name: entry.name.clone(),
                source: entry.source.clone(),
            })
            .collect();
        let kept: Vec<String> = compile_generators(&specs)
            .into_iter()
            .map(|generator| generator.name)
            .collect();
        let accepted: Vec<String> = entries
            .iter()
            .zip(&codes)
            .filter(|(_, code)| **code == 0)
            .map(|(entry, _)| entry.name.clone())
            .collect();
        assert_eq!(kept, accepted);
        assert_eq!(kept, vec!["cpf", "ok", "ação", "futuro", "senha"]);
    }
}
