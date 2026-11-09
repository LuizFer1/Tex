use serde::{Deserialize, Serialize};
use tex_index::{MatchKind, Query};
use tex_ipc::Filter;
use toml::{Table, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub is_app: bool,
}

/// The categories Settings › Search lets the popup show.
#[derive(Debug, Clone, Copy)]
pub struct Shown {
    pub apps: bool,
    pub files: bool,
    pub folders: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorites {
    pub items: Vec<Favorite>,
}

impl Favorite {
    fn fits(&self, filter: Filter, shown: Shown) -> bool {
        let (by_tab, by_settings) = if self.is_app {
            (matches!(filter, Filter::All | Filter::Apps), shown.apps)
        } else if self.is_dir {
            (
                matches!(filter, Filter::All | Filter::Folders),
                shown.folders,
            )
        } else {
            (matches!(filter, Filter::All | Filter::Files), shown.files)
        };
        by_tab && by_settings
    }
}

impl Favorites {
    pub fn contains(&self, path: &str) -> bool {
        self.items
            .iter()
            .any(|item| item.path.eq_ignore_ascii_case(path))
    }

    /// Adds `favorite` at the end, or removes the one with its path. Returns whether it is a favorite now.
    pub fn toggle(&mut self, favorite: Favorite) -> bool {
        let before = self.items.len();
        self.items
            .retain(|item| !item.path.eq_ignore_ascii_case(&favorite.path));
        if self.items.len() < before {
            return false;
        }
        self.items.push(favorite);
        true
    }

    /// The favorites the popup shows with no query, in the order they were added.
    pub fn visible(&self, filter: Filter, shown: Shown) -> Vec<&Favorite> {
        self.items
            .iter()
            .filter(|item| item.fits(filter, shown))
            .collect()
    }

    /// The favorites whose name matches `query`, best match first (ties keep the order they were
    /// added). Queries with a folder or drive match none.
    pub fn matching(&self, query: &str, filter: Filter, shown: Shown) -> Vec<&Favorite> {
        if query.contains(['/', '\\']) {
            return Vec::new();
        }
        let Some(parsed) = Query::parse(query) else {
            return Vec::new();
        };
        let mut scratch = String::new();
        let mut ranked: Vec<(MatchKind, &Favorite)> = self
            .items
            .iter()
            .filter(|item| item.fits(filter, shown))
            .filter_map(|item| {
                parsed
                    .matches(item.name.as_bytes(), &mut scratch)
                    .map(|kind| (kind, item))
            })
            .collect();
        ranked.sort_by_key(|(kind, _)| std::cmp::Reverse(*kind));
        ranked.into_iter().map(|(_, item)| item).collect()
    }

    /// Reads `[[favorite]]` entries; a bad entry is skipped, a file that is not TOML is `None`.
    pub fn from_toml(text: &str) -> Option<Self> {
        let table: Table = text.parse().ok()?;
        let items = match table.get("favorite") {
            Some(Value::Array(entries)) => entries
                .iter()
                .filter_map(|entry| entry.clone().try_into::<Favorite>().ok())
                .filter(|item| !item.path.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        Some(Self { items })
    }

    pub fn to_toml(&self) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            favorite: &'a [Favorite],
        }
        toml::to_string(&File {
            favorite: &self.items,
        })
        .unwrap_or_default()
    }
}

/// `%APPDATA%\Tex\favorites.toml`, beside the settings.
pub fn favorites_path() -> std::path::PathBuf {
    crate::settings::settings_path().with_file_name("favorites.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Shown = Shown {
        apps: true,
        files: true,
        folders: true,
    };

    fn fav(path: &str, name: &str, is_dir: bool, is_app: bool) -> Favorite {
        Favorite {
            path: path.to_owned(),
            name: name.to_owned(),
            is_dir,
            is_app,
        }
    }

    fn names<'a>(found: &[&'a Favorite]) -> Vec<&'a str> {
        found.iter().map(|item| item.name.as_str()).collect()
    }

    #[test]
    fn toggle_adds_at_the_end_and_removes_ignoring_case() {
        let mut favorites = Favorites::default();
        assert!(favorites.toggle(fav(r"C:\a\One.txt", "One.txt", false, false)));
        assert!(favorites.toggle(fav(r"C:\a\Two.txt", "Two.txt", false, false)));
        assert_eq!(favorites.items[0].name, "One.txt");
        assert_eq!(favorites.items[1].name, "Two.txt");
        assert!(favorites.contains(r"c:\A\one.TXT"));
        assert!(!favorites.toggle(fav(r"C:\A\ONE.TXT", "One.txt", false, false)));
        assert!(!favorites.contains(r"C:\a\One.txt"));
        assert_eq!(favorites.items.len(), 1);
    }

    #[test]
    fn visible_filters_by_tab_and_settings() {
        let favorites = Favorites {
            items: vec![
                fav(r"C:\Apps\App.lnk", "App", false, true),
                fav(r"C:\Docs", "Docs", true, false),
                fav(r"C:\a.txt", "a.txt", false, false),
            ],
        };
        assert_eq!(names(&favorites.visible(Filter::Apps, ALL)), ["App"]);
        assert_eq!(names(&favorites.visible(Filter::Folders, ALL)), ["Docs"]);
        assert_eq!(names(&favorites.visible(Filter::Files, ALL)), ["a.txt"]);
        assert_eq!(favorites.visible(Filter::All, ALL).len(), 3);
        let no_files = Shown {
            files: false,
            ..ALL
        };
        assert_eq!(
            names(&favorites.visible(Filter::All, no_files)),
            ["App", "Docs"]
        );
    }

    #[test]
    fn matching_ranks_by_match_and_ignores_accents() {
        let favorites = Favorites {
            items: vec![
                fav(r"C:\Chrome.lnk", "Chrome", false, true),
                fav(r"C:\Handbrake.lnk", "Handbrake", false, true),
                fav(r"C:\Ação.txt", "Ação.txt", false, false),
            ],
        };
        assert_eq!(
            names(&favorites.matching("h", Filter::All, ALL)),
            ["Handbrake", "Chrome"]
        );
        assert_eq!(
            names(&favorites.matching("acao", Filter::All, ALL)),
            ["Ação.txt"]
        );
    }

    #[test]
    fn path_queries_match_nothing() {
        let favorites = Favorites {
            items: vec![fav(r"C:\Handbrake.lnk", "Handbrake", false, true)],
        };
        assert!(favorites.matching(r"c:\h", Filter::All, ALL).is_empty());
        assert!(favorites.matching("docs/h", Filter::All, ALL).is_empty());
    }

    #[test]
    fn toml_round_trip_and_bad_entries_skipped() {
        let favorites = Favorites {
            items: vec![
                fav(r"C:\Apps\App.lnk", "App", false, true),
                fav(r"C:\Docs", "Docs", true, false),
            ],
        };
        assert_eq!(Favorites::from_toml(&favorites.to_toml()), Some(favorites));
        let text = concat!(
            "[[favorite]]\npath = 'C:\\x'\nis_dir = false\nis_app = false\n\n",
            "[[favorite]]\npath = 'C:\\y'\nname = 'y'\nis_dir = false\nis_app = false\n",
        );
        let loaded = Favorites::from_toml(text).unwrap();
        assert_eq!(names(&loaded.items.iter().collect::<Vec<_>>()), ["y"]);
        assert_eq!(Favorites::from_toml("not toml ["), None);
    }
}
