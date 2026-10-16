use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lowercase without accents ("Relatório" -> "relatorio"). Used for both queries and names.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    fold_into(text, &mut out);
    out
}

pub(crate) fn fold_into(text: &str, out: &mut String) {
    out.clear();
    if text.is_ascii() {
        out.extend(text.chars().map(|c| c.to_ascii_lowercase()));
        return;
    }
    for c in text.nfd().filter(|c| !is_combining_mark(*c)) {
        out.extend(c.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_ascii() {
        assert_eq!(fold("README.md"), "readme.md");
    }

    #[test]
    fn strips_accents() {
        assert_eq!(fold("Relatório"), "relatorio");
        assert_eq!(fold("CONFIGURAÇÃO"), "configuracao");
    }

    #[test]
    fn keeps_letters_without_decomposition() {
        assert_eq!(fold("Straße"), "straße");
    }

    #[test]
    fn fold_into_reuses_the_buffer() {
        let mut buffer = String::from("leftover");
        fold_into("Ação", &mut buffer);
        assert_eq!(buffer, "acao");
    }
}
