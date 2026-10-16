use crate::normalize::{fold, fold_into};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    Anywhere,
    WordStart,
    Prefix,
    Exact,
}

#[derive(Debug, Clone)]
pub struct Query {
    terms: Vec<String>,
}

impl Query {
    /// Splits on whitespace and folds each term. `None` when nothing is left to search.
    pub fn parse(text: &str) -> Option<Self> {
        let terms: Vec<String> = text
            .split_whitespace()
            .map(fold)
            .filter(|term| !term.is_empty())
            .collect();
        (!terms.is_empty()).then_some(Self { terms })
    }

    /// Match quality of the first term, if every term occurs in `name` (UTF-8 bytes).
    pub fn matches(&self, name: &[u8], scratch: &mut String) -> Option<MatchKind> {
        let (haystack, camel) = if name.is_ascii() {
            (name, true)
        } else {
            fold_into(std::str::from_utf8(name).ok()?, scratch);
            (scratch.as_bytes(), false)
        };
        let (first, rest) = self.terms.split_first()?;
        let kind = classify(haystack, first.as_bytes(), camel)?;
        rest.iter()
            .all(|term| find(haystack, term.as_bytes(), 0).is_some())
            .then_some(kind)
    }
}

fn classify(haystack: &[u8], term: &[u8], camel: bool) -> Option<MatchKind> {
    if haystack.len() == term.len() && eq_lower(haystack, term) {
        return Some(MatchKind::Exact);
    }
    let mut best = None;
    let mut from = 0;
    while let Some(at) = find(haystack, term, from) {
        let kind = if at == 0 {
            MatchKind::Prefix
        } else if is_word_start(haystack, at, camel) {
            MatchKind::WordStart
        } else {
            MatchKind::Anywhere
        };
        best = best.max(Some(kind));
        if kind >= MatchKind::WordStart {
            break;
        }
        from = at + 1;
    }
    best
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let last = haystack.len().checked_sub(needle.len())?;
    (from..=last).find(|&at| eq_lower(&haystack[at..at + needle.len()], needle))
}

fn eq_lower(bytes: &[u8], lower: &[u8]) -> bool {
    bytes
        .iter()
        .zip(lower)
        .all(|(b, l)| b.to_ascii_lowercase() == *l)
}

fn is_word_start(haystack: &[u8], at: usize, camel: bool) -> bool {
    let previous = haystack[at - 1];
    matches!(previous, b' ' | b'_' | b'-' | b'.')
        || (camel && previous.is_ascii_lowercase() && haystack[at].is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(query: &str, name: &str) -> Option<MatchKind> {
        Query::parse(query)
            .unwrap()
            .matches(name.as_bytes(), &mut String::new())
    }

    #[test]
    fn blank_query_is_none() {
        assert!(Query::parse("   ").is_none());
    }

    #[test]
    fn every_term_must_appear() {
        assert_eq!(
            kind("rel pdf", "Relatorio_final.pdf"),
            Some(MatchKind::Prefix)
        );
        assert_eq!(kind("rel xyz", "Relatorio_final.pdf"), None);
        assert_eq!(kind("xyz", "abc"), None);
    }

    #[test]
    fn classifies_the_first_term() {
        assert_eq!(kind("readme.md", "README.md"), Some(MatchKind::Exact));
        assert_eq!(kind("rel", "Relatorio.pdf"), Some(MatchKind::Prefix));
        assert_eq!(kind("pdf", "Relatorio.pdf"), Some(MatchKind::WordStart));
        assert_eq!(
            kind("final", "Relatorio_final.pdf"),
            Some(MatchKind::WordStart)
        );
        assert_eq!(kind("state", "AppState.rs"), Some(MatchKind::WordStart));
        assert_eq!(kind("lat", "Relatorio.pdf"), Some(MatchKind::Anywhere));
    }

    #[test]
    fn prefers_the_best_occurrence() {
        assert_eq!(kind("log", "catalog log.txt"), Some(MatchKind::WordStart));
    }

    #[test]
    fn ignores_accents_on_both_sides() {
        assert_eq!(kind("relatorio", "Relatório.docx"), Some(MatchKind::Prefix));
        assert_eq!(kind("relatório", "relatorio.docx"), Some(MatchKind::Prefix));
        assert_eq!(kind("cao", "Configuração"), Some(MatchKind::Anywhere));
    }

    #[test]
    fn kinds_are_ordered_from_worst_to_best() {
        assert!(MatchKind::Anywhere < MatchKind::WordStart);
        assert!(MatchKind::WordStart < MatchKind::Prefix);
        assert!(MatchKind::Prefix < MatchKind::Exact);
    }
}
