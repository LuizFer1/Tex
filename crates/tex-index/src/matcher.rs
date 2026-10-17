use crate::normalize::{fold, fold_into};

const MAX_FOLDERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    Anywhere,
    WordStart,
    Prefix,
    Exact,
}

#[derive(Debug, Clone)]
struct Term {
    text: String,
    extension: bool,
}

#[derive(Debug, Clone)]
pub struct Query {
    terms: Vec<Term>,
    folders: Vec<Vec<String>>,
    drive: Option<char>,
}

impl Query {
    /// Splits on `/` and `\`: the last piece is the name and the pieces before it are folders,
    /// outermost first, keeping the 8 nearest the name. A first piece like `C:` picks the drive
    /// instead. Each piece splits on whitespace into folded terms. `None` when nothing is left.
    pub fn parse(text: &str) -> Option<Self> {
        let mut pieces = text.split(['/', '\\']);
        let name = pieces.next_back().unwrap_or_default();
        let mut pieces = pieces
            .map(str::trim)
            .filter(|piece| !piece.is_empty())
            .peekable();
        let drive = pieces
            .next_if(|piece| is_drive(piece))
            .and_then(|piece| piece.chars().next())
            .map(|letter| letter.to_ascii_uppercase());
        let mut folders: Vec<Vec<String>> = pieces
            .map(words)
            .filter(|words| !words.is_empty())
            .collect();
        folders = folders.split_off(folders.len().saturating_sub(MAX_FOLDERS));
        let terms: Vec<Term> = words(name)
            .into_iter()
            .map(|text| Term {
                extension: text.len() > 1 && text.starts_with('.'),
                text,
            })
            .collect();
        (!terms.is_empty() || !folders.is_empty() || drive.is_some()).then_some(Self {
            terms,
            folders,
            drive,
        })
    }

    /// Match quality of the first term, if every term occurs in `name` (UTF-8 bytes). Without
    /// name terms every name matches as `Exact`.
    pub fn matches(&self, name: &[u8], scratch: &mut String) -> Option<MatchKind> {
        let Some((first, rest)) = self.terms.split_first() else {
            return Some(MatchKind::Exact);
        };
        let (haystack, camel) = haystack(name, scratch)?;
        let kind = first.classify(haystack, camel)?;
        rest.iter()
            .all(|term| term.occurs(haystack))
            .then_some(kind)
    }

    pub(crate) fn drive(&self) -> Option<char> {
        self.drive
    }

    pub(crate) fn folder_count(&self) -> usize {
        self.folders.len()
    }

    /// Match quality of folder `index` (0 is the outermost) against a directory name.
    pub(crate) fn folder_kind(
        &self,
        index: usize,
        name: &[u8],
        scratch: &mut String,
    ) -> Option<MatchKind> {
        let (first, rest) = self.folders.get(index)?.split_first()?;
        let (haystack, camel) = haystack(name, scratch)?;
        let kind = classify(haystack, first.as_bytes(), camel)?;
        rest.iter()
            .all(|word| find(haystack, word.as_bytes(), 0).is_some())
            .then_some(kind)
    }
}

impl Term {
    fn classify(&self, haystack: &[u8], camel: bool) -> Option<MatchKind> {
        let text = self.text.as_bytes();
        if !self.extension {
            classify(haystack, text, camel)
        } else if !ends_with(haystack, text) {
            None
        } else if haystack.len() == text.len() {
            Some(MatchKind::Exact)
        } else {
            Some(MatchKind::WordStart)
        }
    }

    fn occurs(&self, haystack: &[u8]) -> bool {
        let text = self.text.as_bytes();
        if self.extension {
            ends_with(haystack, text)
        } else {
            find(haystack, text, 0).is_some()
        }
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

fn words(piece: &str) -> Vec<String> {
    piece
        .split_whitespace()
        .map(fold)
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_drive(piece: &str) -> bool {
    let bytes = piece.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn haystack<'a>(name: &'a [u8], scratch: &'a mut String) -> Option<(&'a [u8], bool)> {
    if name.is_ascii() {
        Some((name, true))
    } else {
        fold_into(std::str::from_utf8(name).ok()?, scratch);
        Some((scratch.as_bytes(), false))
    }
}

fn ends_with(haystack: &[u8], suffix: &[u8]) -> bool {
    haystack.len() >= suffix.len() && eq_lower(&haystack[haystack.len() - suffix.len()..], suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(query: &str, name: &str) -> Option<MatchKind> {
        Query::parse(query)
            .unwrap()
            .matches(name.as_bytes(), &mut String::new())
    }

    fn parts(text: &str) -> (Vec<Vec<String>>, Vec<String>, Option<char>) {
        let query = Query::parse(text).unwrap();
        (
            query.folders,
            query.terms.into_iter().map(|term| term.text).collect(),
            query.drive,
        )
    }

    #[test]
    fn slashes_split_folders_from_the_name() {
        let (folders, terms, drive) = parts("Projetos Pessoais/Tex/.exe");
        assert_eq!(folders, [vec!["projetos", "pessoais"], vec!["tex"]]);
        assert_eq!(terms, [".exe"]);
        assert_eq!(drive, None);
        let (folders, terms, drive) = parts(r"C:\Users\luis\");
        assert_eq!(folders, [vec!["users"], vec!["luis"]]);
        assert!(terms.is_empty());
        assert_eq!(drive, Some('C'));
    }

    #[test]
    fn empty_pieces_are_ignored() {
        assert!(parts("/x").0.is_empty());
        assert_eq!(parts("Tex//x").0, [vec!["tex"]]);
        assert_eq!(parts("Tex/ /x").0, [vec!["tex"]]);
        assert!(Query::parse("/").is_none());
        assert!(Query::parse(" / \\ ").is_none());
    }

    #[test]
    fn drive_only_counts_as_the_first_piece() {
        let (folders, _, drive) = parts("a/c:/x");
        assert_eq!(folders, [vec!["a"], vec!["c:"]]);
        assert_eq!(drive, None);
        assert_eq!(parts("d:/x").2, Some('D'));
        assert_eq!(parts("C:").1, ["c:"]);
    }

    #[test]
    fn keeps_the_eight_folders_nearest_the_name() {
        let folders = parts("1/2/3/4/5/6/7/8/9/10/x").0;
        assert_eq!(folders.len(), 8);
        assert_eq!(folders[0], ["3"]);
        assert_eq!(folders[7], ["10"]);
    }

    #[test]
    fn name_can_be_empty_after_a_folder() {
        let query = Query::parse("tex/").unwrap();
        assert_eq!(
            query.matches(b"anything", &mut String::new()),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn leading_dot_matches_the_extension() {
        assert_eq!(kind(".exe", "tex.exe"), Some(MatchKind::WordStart));
        assert_eq!(kind(".exe", "TEX.EXE"), Some(MatchKind::WordStart));
        assert_eq!(kind(".exe", ".exe"), Some(MatchKind::Exact));
        assert_eq!(kind(".exe", "tex.exe.config"), None);
        assert_eq!(kind(".tar.gz", "a.tar.gz"), Some(MatchKind::WordStart));
        assert_eq!(kind("rel .pdf", "Relatorio.pdf"), Some(MatchKind::Prefix));
        assert_eq!(kind("rel .pdf", "Relatorio.pdf.bak"), None);
        assert_eq!(kind(".", "a.b"), Some(MatchKind::Anywhere));
    }

    #[test]
    fn folders_match_like_names() {
        let query = Query::parse("projetos pessoais/tex/x").unwrap();
        let folder =
            |index, name: &str| query.folder_kind(index, name.as_bytes(), &mut String::new());
        assert_eq!(folder(0, "Projetos Pessoais"), Some(MatchKind::Prefix));
        assert_eq!(folder(0, "Projetos"), None);
        assert_eq!(folder(1, "Tex"), Some(MatchKind::Exact));
        assert_eq!(folder(1, "Latex"), Some(MatchKind::Anywhere));
        assert_eq!(folder(1, "Configuração"), None);
        assert_eq!(folder(2, "x"), None);
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
