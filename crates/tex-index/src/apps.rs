use crate::matcher::{MatchKind, Query};

/// An app from a `.desktop` file (freedesktop Desktop Entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    /// The desktop file id, such as `firefox.desktop`.
    pub id: String,
    pub path: String,
    /// False for `NoDisplay=true` and `Hidden=true`, which still hide an app with the same id.
    pub shown: bool,
    name: Localized,
    generic_name: Localized,
    keywords: Localized,
}

impl DesktopApp {
    /// Reads the `[Desktop Entry]` group of `text`. `None` unless it is an application with a
    /// name, or a `Hidden=true` entry (which only hides apps with the same id).
    pub fn parse(id: &str, path: &str, text: &str) -> Option<Self> {
        let mut in_entry = false;
        let mut application = false;
        let mut no_display = false;
        let mut hidden = false;
        let mut name = Localized::default();
        let mut generic_name = Localized::default();
        let mut keywords = Localized::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else {
                continue;
            };
            let (field, locale) = match key.trim().split_once('[') {
                Some((field, rest)) => match rest.strip_suffix(']') {
                    Some(locale) => (field, Some(locale)),
                    None => continue,
                },
                None => (key.trim(), None),
            };
            let value = unescape(value.trim());
            match (field, locale) {
                ("Type", None) => application = value == "Application",
                ("NoDisplay", None) => no_display = value == "true",
                ("Hidden", None) => hidden = value == "true",
                ("Name", _) => name.set(locale, value),
                ("GenericName", _) => generic_name.set(locale, value),
                ("Keywords", _) => keywords.set(locale, words(&value)),
                _ => {}
            }
        }
        (hidden || (application && name.default.is_some())).then(|| Self {
            id: id.to_owned(),
            path: path.to_owned(),
            shown: !hidden && !no_display,
            name,
            generic_name,
            keywords,
        })
    }

    /// The name in `language` ("pt-BR"), falling back to the plain `Name`.
    pub fn name(&self, language: &str) -> &str {
        self.name.get(language).unwrap_or_default()
    }

    /// `(0, kind)` when `query` matches the name (in `language` or plain), `(1, kind)` when it
    /// matches only the generic name or the keywords. `scratch` is reused between calls.
    pub fn rank(
        &self,
        query: &Query,
        language: &str,
        scratch: &mut String,
    ) -> Option<(u8, MatchKind)> {
        let names = [self.name.get(language), self.name.default.as_deref()];
        if let Some(kind) = best(query, names, scratch) {
            return Some((0, kind));
        }
        let others = [self.generic_name.get(language), self.keywords.get(language)];
        best(query, others, scratch).map(|kind| (1, kind))
    }
}

fn best(query: &Query, texts: [Option<&str>; 2], scratch: &mut String) -> Option<MatchKind> {
    texts
        .into_iter()
        .flatten()
        .filter_map(|text| query.matches(text.as_bytes(), scratch))
        .max()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Localized {
    default: Option<String>,
    translations: Vec<(String, String)>,
}

impl Localized {
    fn set(&mut self, locale: Option<&str>, value: String) {
        match locale {
            Some(locale) => self.translations.push((locale.to_owned(), value)),
            None => self.default = Some(value),
        }
    }

    /// The value for `language` ("pt-BR" or "pt_BR"): that locale, then the language alone
    /// ("pt"), then the plain value. Locale modifiers ("sr@latin") are ignored.
    fn get(&self, language: &str) -> Option<&str> {
        let language = language.replace('-', "_");
        let base = language.split('_').next().unwrap_or_default();
        [language.as_str(), base]
            .into_iter()
            .filter(|wanted| !wanted.is_empty())
            .find_map(|wanted| {
                self.translations
                    .iter()
                    .find(|(locale, _)| without_modifier(locale) == wanted)
                    .map(|(_, value)| value.as_str())
            })
            .or(self.default.as_deref())
    }
}

fn without_modifier(locale: &str) -> &str {
    locale.split(['@', '.']).next().unwrap_or(locale)
}

/// `Keywords=a;b;c;` as "a b c", so a query with several words can match across keywords.
fn words(value: &str) -> String {
    value
        .split(';')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Desktop entry escapes: `\s`, `\n`, `\t`, `\r` and `\\`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "[Desktop Entry]\nVersion=1.0\nType=Application\nName=Firefox\nName[pt_BR]=Navegador Firefox\nGenericName=Web Browser\nGenericName[pt]=Navegador Web\nKeywords=Internet;WWW;Browser;\nExec=firefox %u\n\n[Desktop Action new-window]\nName=New Window\n";

    #[test]
    fn reads_the_desktop_entry_group() {
        let app = DesktopApp::parse(
            "firefox.desktop",
            "/usr/share/applications/firefox.desktop",
            FIREFOX,
        )
        .unwrap();
        assert_eq!(app.id, "firefox.desktop");
        assert_eq!(app.path, "/usr/share/applications/firefox.desktop");
        assert!(app.shown);
        assert_eq!(app.name(""), "Firefox");
        assert_eq!(app.name("pt-BR"), "Navegador Firefox");
        assert_eq!(app.name("pt_BR"), "Navegador Firefox");
        assert_eq!(app.name("de-DE"), "Firefox");
        assert_eq!(app.generic_name.get("pt-BR"), Some("Navegador Web"));
        assert_eq!(app.keywords.get(""), Some("Internet WWW Browser"));
    }

    #[test]
    fn skips_what_is_not_a_shown_application() {
        let link = "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.com\n";
        assert!(DesktopApp::parse("site.desktop", "/x/site.desktop", link).is_none());
        let nameless = "[Desktop Entry]\nType=Application\nExec=x\n";
        assert!(DesktopApp::parse("x.desktop", "/x/x.desktop", nameless).is_none());
        let other_group = "[Other]\nType=Application\nName=X\n";
        assert!(DesktopApp::parse("x.desktop", "/x/x.desktop", other_group).is_none());
        let no_display = "[Desktop Entry]\nType=Application\nName=Helper\nNoDisplay=true\n";
        assert!(
            !DesktopApp::parse("h.desktop", "/x/h.desktop", no_display)
                .unwrap()
                .shown
        );
        let deleted = "[Desktop Entry]\nHidden=true\n";
        assert!(
            !DesktopApp::parse("h.desktop", "/x/h.desktop", deleted)
                .unwrap()
                .shown
        );
    }

    #[test]
    fn unescapes_values_and_ignores_comments() {
        let text = "# comment\n[Desktop Entry]\nType = Application\nName=Tex\\sApp\\\\x\nName[sr@latin]=Teks\n";
        let app = DesktopApp::parse("t.desktop", "/x/t.desktop", text).unwrap();
        assert_eq!(app.name(""), r"Tex App\x");
        assert_eq!(app.name("sr"), "Teks");
    }

    #[test]
    fn rank_prefers_the_name() {
        let app = DesktopApp::parse("firefox.desktop", "/x/firefox.desktop", FIREFOX).unwrap();
        let rank = |text: &str, language: &str| {
            app.rank(
                &Query::parse_for(text, false).unwrap(),
                language,
                &mut String::new(),
            )
        };
        assert_eq!(rank("fire", ""), Some((0, MatchKind::Prefix)));
        assert_eq!(rank("browser", ""), Some((1, MatchKind::WordStart)));
        assert_eq!(rank("navegador", "pt-BR"), Some((0, MatchKind::Prefix)));
        assert_eq!(rank("www", ""), Some((1, MatchKind::WordStart)));
        assert_eq!(rank("chrome", ""), None);
    }
}
