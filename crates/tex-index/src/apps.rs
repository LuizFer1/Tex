use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use crate::matcher::{MatchKind, Query};
use crate::normalize::fold;
use crate::search::Hit;

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

type AppKey = (u8, Reverse<MatchKind>, usize, String);

/// The apps the search offers: the system's `.desktop` files and each user's own.
#[derive(Debug, Clone, Default)]
pub struct AppCatalog {
    system: Vec<DesktopApp>,
    users: HashMap<String, Vec<DesktopApp>>,
}

impl AppCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apps from the system folders, the most important folder first: the first app with an id wins.
    pub fn set_system(&mut self, apps: Vec<DesktopApp>) {
        self.system = first_of_each_id(apps);
    }

    /// The apps of the user whose home is `home`; they hide system apps with the same id.
    pub fn set_user(&mut self, home: &str, apps: Vec<DesktopApp>) {
        self.users.insert(home.to_owned(), first_of_each_id(apps));
    }

    pub fn is_empty(&self) -> bool {
        self.system.is_empty() && self.users.values().all(Vec::is_empty)
    }

    /// Shown apps of the user at `home` that match `query`, best first, labeled in `language`.
    pub(crate) fn search(&self, query: &Query, language: &str, home: Option<&str>) -> Vec<Hit> {
        let user = home
            .and_then(|home| self.users.get(home))
            .map_or(&[][..], Vec::as_slice);
        let mine: HashSet<&str> = user.iter().map(|app| app.id.as_str()).collect();
        let system = self
            .system
            .iter()
            .filter(|app| !mine.contains(app.id.as_str()));
        let mut scratch = String::new();
        let mut found: Vec<(AppKey, Hit)> = user
            .iter()
            .chain(system)
            .filter(|app| app.shown)
            .filter_map(|app| {
                let (field, kind) = app.rank(query, language, &mut scratch)?;
                let label = app.name(language).to_owned();
                let key = (field, Reverse(kind), label.len(), fold(&label));
                let hit = Hit {
                    path: app.path.clone(),
                    is_dir: false,
                    is_app: true,
                    label: Some(label),
                };
                Some((key, hit))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.into_iter().map(|(_, hit)| hit).collect()
    }
}

fn first_of_each_id(apps: Vec<DesktopApp>) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    apps.into_iter()
        .filter(|app| seen.insert(app.id.clone()))
        .collect()
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

    fn app(id: &str, text: &str) -> DesktopApp {
        DesktopApp::parse(id, &format!("/usr/share/applications/{id}"), text).unwrap()
    }

    fn entry(name: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\n{extra}")
    }

    fn labels(catalog: &AppCatalog, text: &str, language: &str, home: Option<&str>) -> Vec<String> {
        let query = Query::parse_for(text, false).unwrap();
        catalog
            .search(&query, language, home)
            .into_iter()
            .map(|hit| hit.label.unwrap())
            .collect()
    }

    #[test]
    fn names_rank_above_generic_names_and_keywords() {
        let mut catalog = AppCatalog::new();
        catalog.set_system(vec![
            app(
                "files.desktop",
                &entry("Files", "Keywords=folder;browser;\n"),
            ),
            app("firefox.desktop", FIREFOX),
            app("brow.desktop", &entry("Browse Helper", "")),
        ]);
        assert_eq!(
            labels(&catalog, "brow", "", None),
            ["Browse Helper", "Files", "Firefox"]
        );
        let hits = catalog.search(&Query::parse_for("fire", false).unwrap(), "", None);
        assert_eq!(hits[0].path, "/usr/share/applications/firefox.desktop");
        assert!(hits[0].is_app);
        assert!(!hits[0].is_dir);
    }

    #[test]
    fn labels_follow_the_language() {
        let mut catalog = AppCatalog::new();
        catalog.set_system(vec![app("firefox.desktop", FIREFOX)]);
        assert_eq!(
            labels(&catalog, "navegador", "pt-BR", None),
            ["Navegador Firefox"]
        );
        assert_eq!(
            labels(&catalog, "firefox", "pt-BR", None),
            ["Navegador Firefox"]
        );
        assert!(labels(&catalog, "navegador", "en-US", None).is_empty());
    }

    #[test]
    fn hidden_apps_and_other_users_stay_out() {
        let mut catalog = AppCatalog::new();
        catalog.set_system(vec![
            app("tool.desktop", &entry("Tool", "")),
            app("tool.desktop", &entry("Tool Copy", "")),
            app("secret.desktop", &entry("Tool Secret", "NoDisplay=true\n")),
            app("editor.desktop", &entry("Editor Tool", "")),
        ]);
        catalog.set_user(
            "/home/ana",
            vec![
                app("mine.desktop", &entry("My Tool", "")),
                app("editor.desktop", "[Desktop Entry]\nHidden=true\n"),
            ],
        );
        assert_eq!(labels(&catalog, "tool", "", None), ["Tool", "Editor Tool"]);
        assert_eq!(
            labels(&catalog, "tool", "", Some("/home/ana")),
            ["Tool", "My Tool"]
        );
        assert_eq!(
            labels(&catalog, "tool", "", Some("/home/bia")),
            ["Tool", "Editor Tool"]
        );
        assert!(!catalog.is_empty());
        assert!(AppCatalog::new().is_empty());
    }
}
