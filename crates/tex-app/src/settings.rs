use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::shortcut::Shortcut;

pub const DEFAULT_HOTKEY: &str = "alt+space";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub general: General,
    pub search: Search,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct General {
    pub language: Language,
    pub theme: Theme,
    pub hotkey: String,
    pub windows_key: bool,
    pub start_with_windows: bool,
    pub popup_monitor: PopupMonitor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Search {
    pub keep_last_search: KeepSearch,
    pub include_hidden: bool,
    pub include_system: bool,
    pub excluded_folders: Vec<String>,
    pub show_apps: bool,
    pub show_files: bool,
    pub show_folders: bool,
    pub enter_action: EnterAction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "en")]
    En,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopupMonitor {
    #[default]
    Cursor,
    Primary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeepSearch {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "30s")]
    Seconds30,
    #[default]
    #[serde(rename = "1m")]
    Minute1,
    #[serde(rename = "5m")]
    Minutes5,
    #[serde(rename = "always")]
    Always,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterAction {
    #[default]
    Open,
    Reveal,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: Language::System,
            theme: Theme::System,
            hotkey: DEFAULT_HOTKEY.into(),
            windows_key: false,
            start_with_windows: true,
            popup_monitor: PopupMonitor::Cursor,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            keep_last_search: KeepSearch::Minute1,
            include_hidden: false,
            include_system: false,
            excluded_folders: Vec::new(),
            show_apps: true,
            show_files: true,
            show_folders: true,
            enter_action: EnterAction::Open,
        }
    }
}

impl KeepSearch {
    /// Whether a search closed at `closed_at` should still be on screen at `now`.
    pub fn keeps(self, closed_at: Instant, now: Instant) -> bool {
        let limit = match self {
            KeepSearch::Never => return false,
            KeepSearch::Always => return true,
            KeepSearch::Seconds30 => Duration::from_secs(30),
            KeepSearch::Minute1 => Duration::from_secs(60),
            KeepSearch::Minutes5 => Duration::from_secs(300),
        };
        now.saturating_duration_since(closed_at) <= limit
    }
}

impl Settings {
    /// Reads every field on its own: a missing or unknown value falls back to its default.
    /// `None` only when the text is not TOML at all.
    pub fn from_toml(text: &str) -> Option<Settings> {
        let table: Table = toml::from_str(text).ok()?;
        let general = table.get("general").and_then(Value::as_table);
        let search = table.get("search").and_then(Value::as_table);
        let g = General::default();
        let s = Search::default();
        let mut settings = Settings {
            general: General {
                language: field(general, "language").unwrap_or(g.language),
                theme: field(general, "theme").unwrap_or(g.theme),
                hotkey: field(general, "hotkey").unwrap_or(g.hotkey),
                windows_key: field(general, "windows_key").unwrap_or(g.windows_key),
                start_with_windows: field(general, "start_with_windows")
                    .unwrap_or(g.start_with_windows),
                popup_monitor: field(general, "popup_monitor").unwrap_or(g.popup_monitor),
            },
            search: Search {
                keep_last_search: field(search, "keep_last_search").unwrap_or(s.keep_last_search),
                include_hidden: field(search, "include_hidden").unwrap_or(s.include_hidden),
                include_system: field(search, "include_system").unwrap_or(s.include_system),
                excluded_folders: field(search, "excluded_folders").unwrap_or(s.excluded_folders),
                show_apps: field(search, "show_apps").unwrap_or(s.show_apps),
                show_files: field(search, "show_files").unwrap_or(s.show_files),
                show_folders: field(search, "show_folders").unwrap_or(s.show_folders),
                enter_action: field(search, "enter_action").unwrap_or(s.enter_action),
            },
        };
        settings.normalize();
        Some(settings)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("settings always serialize")
    }

    /// Canonical hotkey, clean folder list and at least one category shown.
    pub fn normalize(&mut self) {
        self.general.hotkey = Shortcut::parse(&self.general.hotkey).map_or_else(
            || DEFAULT_HOTKEY.to_owned(),
            |shortcut| shortcut.canonical(),
        );
        self.search.excluded_folders = normalize_folders(&self.search.excluded_folders);
        let search = &mut self.search;
        if !search.show_apps && !search.show_files && !search.show_folders {
            search.show_apps = true;
            search.show_files = true;
            search.show_folders = true;
        }
    }
}

fn field<T: DeserializeOwned>(table: Option<&Table>, key: &str) -> Option<T> {
    table?.get(key)?.clone().try_into().ok()
}

fn normalize_folders(folders: &[String]) -> Vec<String> {
    let mut clean: Vec<String> = Vec::new();
    for folder in folders {
        let folder = folder.trim();
        let folder = if folder.len() > 3 {
            folder.trim_end_matches(['\\', '/'])
        } else {
            folder
        };
        if folder.is_empty() || clean.iter().any(|seen| seen.eq_ignore_ascii_case(folder)) {
            continue;
        }
        clean.push(folder.to_owned());
    }
    clean
}

pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                .join("AppData")
                .join("Roaming")
        });
    base.join("Tex").join("settings.toml")
}

/// Missing file: defaults. Unreadable or not TOML: defaults, and the file is kept as `settings.toml.bad`.
pub fn load(path: &Path) -> Settings {
    match fs::read_to_string(path) {
        Ok(text) => Settings::from_toml(&text).unwrap_or_else(|| {
            set_aside(path);
            Settings::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Settings::default(),
        Err(_) => {
            set_aside(path);
            Settings::default()
        }
    }
}

fn set_aside(path: &Path) {
    let _ = fs::rename(path, path.with_extension("toml.bad"));
}

/// Writes to a temporary file and renames it, so a crash never leaves half a file.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, settings.to_toml())?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let settings = Settings::default();
        assert_eq!(settings.general.hotkey, "alt+space");
        assert!(settings.general.start_with_windows);
        assert!(!settings.general.windows_key);
        assert_eq!(settings.search.keep_last_search, KeepSearch::Minute1);
        assert!(!settings.search.include_hidden && !settings.search.include_system);
        assert!(settings.search.show_apps && settings.search.show_files);
        assert_eq!(settings.search.enter_action, EnterAction::Open);
    }

    #[test]
    fn round_trips_through_toml() {
        let mut settings = Settings::default();
        settings.general.language = Language::PtBr;
        settings.general.theme = Theme::Dark;
        settings.general.hotkey = "ctrl+shift+k".into();
        settings.general.popup_monitor = PopupMonitor::Primary;
        settings.search.keep_last_search = KeepSearch::Seconds30;
        settings.search.excluded_folders = vec![r"C:\Temp".into()];
        settings.search.show_files = false;
        settings.search.enter_action = EnterAction::Reveal;
        let text = settings.to_toml();
        assert!(text.contains(r#"language = "pt-BR""#), "{text}");
        assert!(text.contains(r#"keep_last_search = "30s""#), "{text}");
        assert_eq!(Settings::from_toml(&text), Some(settings));
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let settings = Settings::from_toml("[search]\ninclude_hidden = true\n").unwrap();
        assert!(settings.search.include_hidden);
        assert_eq!(settings.general, General::default());
    }

    #[test]
    fn unknown_values_fall_back_per_field() {
        let text = "[general]\ntheme = \"purple\"\nwindows_key = true\nhotkey = \"nope\"\n";
        let settings = Settings::from_toml(text).unwrap();
        assert_eq!(settings.general.theme, Theme::System);
        assert!(settings.general.windows_key);
        assert_eq!(settings.general.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn invalid_toml_is_none() {
        assert_eq!(Settings::from_toml("[general\nx ="), None);
    }

    #[test]
    fn excluded_folders_are_normalized() {
        let folders = [r"C:\A\", r"c:\a", "", r" D:\B ", r"E:\"];
        let text = format!(
            "[search]\nexcluded_folders = [{}]\n",
            folders
                .iter()
                .map(|folder| format!("'{folder}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let settings = Settings::from_toml(&text).unwrap();
        assert_eq!(settings.search.excluded_folders, [r"C:\A", r"D:\B", r"E:\"]);
    }

    #[test]
    fn all_categories_off_turns_them_back_on() {
        let text = "[search]\nshow_apps = false\nshow_files = false\nshow_folders = false\n";
        let search = Settings::from_toml(text).unwrap().search;
        assert!(search.show_apps && search.show_files && search.show_folders);
    }

    #[test]
    fn keeps_search_within_the_limit() {
        let closed = Instant::now();
        let after = |seconds| closed + Duration::from_secs(seconds);
        assert!(!KeepSearch::Never.keeps(closed, closed));
        assert!(KeepSearch::Seconds30.keeps(closed, after(30)));
        assert!(!KeepSearch::Seconds30.keeps(closed, after(31)));
        assert!(KeepSearch::Minute1.keeps(closed, after(60)));
        assert!(!KeepSearch::Minute1.keeps(closed, after(61)));
        assert!(KeepSearch::Minutes5.keeps(closed, after(300)));
        assert!(KeepSearch::Always.keeps(closed, after(86_400)));
    }

    #[test]
    fn load_sets_a_broken_file_aside() {
        let folder = std::env::temp_dir().join(format!("tex-settings-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.toml");
        assert_eq!(load(&path), Settings::default());
        fs::write(&path, "not [ toml").unwrap();
        assert_eq!(load(&path), Settings::default());
        assert!(!path.exists());
        assert!(folder.join("settings.toml.bad").exists());
        let mut settings = Settings::default();
        settings.search.include_system = true;
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
        fs::remove_dir_all(&folder).unwrap();
    }
}
