use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tex_clipboard::{CaptureSettings, Retention};
use toml::{Table, Value};

use crate::shortcut::Shortcut;

pub const DEFAULT_HOTKEY: &str = "alt+space";
pub const DEFAULT_HISTORY_HOTKEY: &str = "alt+v";
const FALLBACK_HISTORY_HOTKEY: &str = "ctrl+shift+v";
pub const ITEM_SIZES_MB: [u32; 5] = [1, 5, 10, 25, 50];
pub const CLEANUP_DAYS: [u32; 3] = [3, 7, 30];
pub const STORAGE_LIMITS_MB: [u32; 5] = [250, 500, 1024, 2048, 5120];
const MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub general: General,
    pub search: Search,
    pub clipboard: Clipboard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct General {
    pub language: Language,
    pub theme: Theme,
    pub hotkey: String,
    pub windows_key: bool,
    pub start_with_windows: bool,
    pub popup_monitor: PopupMonitor,
    pub auto_update: bool,
    pub remote_images: bool,
    pub onboarding_done: bool,
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
    pub calculator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Clipboard {
    pub hotkey: String,
    pub paused: bool,
    pub capture_text: bool,
    pub capture_images: bool,
    pub max_item_mb: u32,
    pub on_pick: OnPick,
    pub cleanup: Cleanup,
    pub cleanup_days: u32,
    pub cleanup_limit_mb: u32,
    pub respect_exclusion: bool,
    pub ignored_apps: Vec<String>,
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
    System,
    #[default]
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
            theme: Theme::Light,
            hotkey: DEFAULT_HOTKEY.into(),
            windows_key: true,
            start_with_windows: true,
            popup_monitor: PopupMonitor::Cursor,
            auto_update: true,
            remote_images: false,
            onboarding_done: false,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            keep_last_search: KeepSearch::Minute1,
            include_hidden: true,
            include_system: false,
            excluded_folders: Vec::new(),
            show_apps: true,
            show_files: true,
            show_folders: true,
            enter_action: EnterAction::Open,
            calculator: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnPick {
    #[default]
    Copy,
    CopyAndPaste,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cleanup {
    #[default]
    Time,
    Storage,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HISTORY_HOTKEY.into(),
            paused: false,
            capture_text: true,
            capture_images: true,
            max_item_mb: 10,
            on_pick: OnPick::Copy,
            cleanup: Cleanup::Time,
            cleanup_days: 7,
            cleanup_limit_mb: 1024,
            respect_exclusion: true,
            ignored_apps: Vec::new(),
        }
    }
}

impl Clipboard {
    pub fn retention(&self) -> Retention {
        match self.cleanup {
            Cleanup::Time => Retention::Time {
                days: self.cleanup_days,
            },
            Cleanup::Storage => Retention::Storage {
                limit_bytes: u64::from(self.cleanup_limit_mb) * MB,
            },
        }
    }

    pub fn capture_settings(&self) -> CaptureSettings {
        CaptureSettings {
            paused: self.paused,
            capture_text: self.capture_text,
            capture_images: self.capture_images,
            max_item_bytes: u64::from(self.max_item_mb) * MB,
            respect_exclusion: self.respect_exclusion,
            ignored_apps: self.ignored_apps.clone(),
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
        let clipboard = table.get("clipboard").and_then(Value::as_table);
        let g = General::default();
        let s = Search::default();
        let c = Clipboard::default();
        let mut settings = Settings {
            general: General {
                language: field(general, "language").unwrap_or(g.language),
                theme: field(general, "theme").unwrap_or(g.theme),
                hotkey: field(general, "hotkey").unwrap_or(g.hotkey),
                windows_key: field(general, "windows_key").unwrap_or(g.windows_key),
                start_with_windows: field(general, "start_with_windows")
                    .unwrap_or(g.start_with_windows),
                popup_monitor: field(general, "popup_monitor").unwrap_or(g.popup_monitor),
                auto_update: field(general, "auto_update").unwrap_or(g.auto_update),
                remote_images: field(general, "remote_images").unwrap_or(g.remote_images),
                onboarding_done: field(general, "onboarding_done").unwrap_or(g.onboarding_done),
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
                calculator: field(search, "calculator").unwrap_or(s.calculator),
            },
            clipboard: Clipboard {
                hotkey: field(clipboard, "hotkey").unwrap_or(c.hotkey),
                paused: field(clipboard, "paused").unwrap_or(c.paused),
                capture_text: field(clipboard, "capture_text").unwrap_or(c.capture_text),
                capture_images: field(clipboard, "capture_images").unwrap_or(c.capture_images),
                max_item_mb: field(clipboard, "max_item_mb").unwrap_or(c.max_item_mb),
                on_pick: field(clipboard, "on_pick").unwrap_or(c.on_pick),
                cleanup: field(clipboard, "cleanup").unwrap_or(c.cleanup),
                cleanup_days: field(clipboard, "cleanup_days").unwrap_or(c.cleanup_days),
                cleanup_limit_mb: field(clipboard, "cleanup_limit_mb")
                    .unwrap_or(c.cleanup_limit_mb),
                respect_exclusion: field(clipboard, "respect_exclusion")
                    .unwrap_or(c.respect_exclusion),
                ignored_apps: field(clipboard, "ignored_apps").unwrap_or(c.ignored_apps),
            },
        };
        settings.normalize();
        Some(settings)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("settings always serialize")
    }

    /// The Windows key hook only runs once the onboarding is done.
    pub fn winkey_active(&self) -> bool {
        self.general.windows_key && self.general.onboarding_done
    }

    /// Canonical hotkeys that differ, clean folder and app lists, values from the fixed lists
    /// and at least one category shown and captured.
    pub fn normalize(&mut self) {
        self.general.hotkey = canonical_hotkey(&self.general.hotkey, DEFAULT_HOTKEY);
        self.search.excluded_folders = normalize_folders(&self.search.excluded_folders);
        let search = &mut self.search;
        if !search.show_apps && !search.show_files && !search.show_folders {
            search.show_apps = true;
            search.show_files = true;
            search.show_folders = true;
        }
        let c = Clipboard::default();
        let clipboard = &mut self.clipboard;
        clipboard.hotkey = canonical_hotkey(&clipboard.hotkey, DEFAULT_HISTORY_HOTKEY);
        if clipboard.hotkey == self.general.hotkey {
            clipboard.hotkey = if self.general.hotkey == DEFAULT_HISTORY_HOTKEY {
                FALLBACK_HISTORY_HOTKEY
            } else {
                DEFAULT_HISTORY_HOTKEY
            }
            .to_owned();
        }
        if !ITEM_SIZES_MB.contains(&clipboard.max_item_mb) {
            clipboard.max_item_mb = c.max_item_mb;
        }
        if !CLEANUP_DAYS.contains(&clipboard.cleanup_days) {
            clipboard.cleanup_days = c.cleanup_days;
        }
        if !STORAGE_LIMITS_MB.contains(&clipboard.cleanup_limit_mb) {
            clipboard.cleanup_limit_mb = c.cleanup_limit_mb;
        }
        clipboard.ignored_apps = normalize_apps(&clipboard.ignored_apps);
        if !clipboard.capture_text && !clipboard.capture_images {
            clipboard.capture_text = true;
            clipboard.capture_images = true;
        }
    }
}

fn field<T: DeserializeOwned>(table: Option<&Table>, key: &str) -> Option<T> {
    table?.get(key)?.clone().try_into().ok()
}

fn canonical_hotkey(text: &str, default: &str) -> String {
    Shortcut::parse(text).map_or_else(|| default.to_owned(), |shortcut| shortcut.canonical())
}

/// Only the file name of each app, without blanks or case-insensitive duplicates.
fn normalize_apps(apps: &[String]) -> Vec<String> {
    let mut clean: Vec<String> = Vec::new();
    for app in apps {
        let app = app.trim();
        let name = app.rsplit(['\\', '/']).next().unwrap_or(app).trim();
        if name.is_empty() || clean.iter().any(|seen| tex_clipboard::same_app(seen, name)) {
            continue;
        }
        clean.push(name.to_owned());
    }
    clean
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

/// The folder of the reader's extensions; debug builds take `TEX_EXTENSIONS_DIR` to test another one.
pub fn extensions_dir() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(dir) = std::env::var_os("TEX_EXTENSIONS_DIR")
    {
        return PathBuf::from(dir);
    }
    settings_path().with_file_name("extensions")
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
    fn calculator_is_on_by_default_and_can_be_turned_off() {
        assert!(Settings::default().search.calculator);
        let settings = Settings::from_toml(
            "[search]
calculator = false
",
        )
        .unwrap();
        assert!(!settings.search.calculator);
    }

    #[test]
    fn defaults_match_the_spec() {
        let settings = Settings::default();
        assert_eq!(settings.general.hotkey, "alt+space");
        assert_eq!(settings.general.theme, Theme::Light);
        assert!(settings.general.start_with_windows);
        assert!(settings.general.windows_key);
        assert!(settings.general.auto_update);
        assert!(!settings.general.onboarding_done);
        assert_eq!(settings.search.keep_last_search, KeepSearch::Minute1);
        assert!(settings.search.include_hidden && !settings.search.include_system);
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
        let settings = Settings::from_toml("[search]\ninclude_hidden = false\n").unwrap();
        assert!(!settings.search.include_hidden);
        assert_eq!(settings.general, General::default());
    }

    #[test]
    fn unknown_values_fall_back_per_field() {
        let text = "[general]\ntheme = \"purple\"\nwindows_key = false\nhotkey = \"nope\"\n";
        let settings = Settings::from_toml(text).unwrap();
        assert_eq!(settings.general.theme, Theme::Light);
        assert!(!settings.general.windows_key);
        assert_eq!(settings.general.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn onboarding_flags_are_read_field_by_field() {
        let settings = Settings::from_toml("[general]\ntheme = \"dark\"\n").unwrap();
        assert!(!settings.general.onboarding_done);
        assert!(settings.general.auto_update);
        let text = "[general]\nonboarding_done = true\nauto_update = false\n";
        let settings = Settings::from_toml(text).unwrap();
        assert!(settings.general.onboarding_done);
        assert!(!settings.general.auto_update);
        assert!(settings.to_toml().contains("onboarding_done = true"));
    }

    #[test]
    fn remote_images_default_off_and_bad_values_fall_back() {
        assert!(!Settings::default().general.remote_images);
        let on = Settings::from_toml("[general]\nremote_images = true\n").unwrap();
        assert!(on.general.remote_images);
        let bad = Settings::from_toml("[general]\nremote_images = \"sim\"\n").unwrap();
        assert!(!bad.general.remote_images);
    }

    #[test]
    fn windows_key_waits_for_the_onboarding() {
        let mut settings = Settings::default();
        assert!(!settings.winkey_active());
        settings.general.onboarding_done = true;
        assert!(settings.winkey_active());
        settings.general.windows_key = false;
        assert!(!settings.winkey_active());
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
    fn clipboard_defaults_match_the_spec() {
        let clipboard = Settings::default().clipboard;
        assert_eq!(clipboard.hotkey, "alt+v");
        assert!(!clipboard.paused);
        assert!(clipboard.capture_text && clipboard.capture_images);
        assert_eq!(clipboard.max_item_mb, 10);
        assert_eq!(clipboard.on_pick, OnPick::Copy);
        assert_eq!(clipboard.cleanup, Cleanup::Time);
        assert_eq!(clipboard.cleanup_days, 7);
        assert_eq!(clipboard.cleanup_limit_mb, 1024);
        assert!(clipboard.respect_exclusion);
        assert!(clipboard.ignored_apps.is_empty());
        assert_eq!(Settings::from_toml("").unwrap().clipboard, clipboard);
    }

    #[test]
    fn clipboard_round_trips_through_toml() {
        let settings = Settings {
            clipboard: Clipboard {
                hotkey: "ctrl+alt+h".into(),
                paused: true,
                capture_text: false,
                capture_images: true,
                max_item_mb: 50,
                on_pick: OnPick::CopyAndPaste,
                cleanup: Cleanup::Storage,
                cleanup_days: 30,
                cleanup_limit_mb: 5120,
                respect_exclusion: false,
                ignored_apps: vec!["KeePass.exe".into()],
            },
            ..Settings::default()
        };
        let text = settings.to_toml();
        assert!(text.contains("[clipboard]"), "{text}");
        assert!(text.contains(r#"on_pick = "copy_and_paste""#), "{text}");
        assert!(text.contains(r#"cleanup = "storage""#), "{text}");
        assert_eq!(Settings::from_toml(&text), Some(settings));
    }

    #[test]
    fn clipboard_values_outside_the_lists_fall_back() {
        let text = "[clipboard]\nmax_item_mb = 7\ncleanup_days = 2\ncleanup_limit_mb = 300\n\
                    on_pick = \"x\"\ncleanup = \"never\"\npaused = true\n";
        let clipboard = Settings::from_toml(text).unwrap().clipboard;
        assert_eq!(clipboard.max_item_mb, 10);
        assert_eq!(clipboard.cleanup_days, 7);
        assert_eq!(clipboard.cleanup_limit_mb, 1024);
        assert_eq!(clipboard.on_pick, OnPick::Copy);
        assert_eq!(clipboard.cleanup, Cleanup::Time);
        assert!(clipboard.paused);
    }

    #[test]
    fn ignored_apps_are_normalized() {
        let text = r"[clipboard]
ignored_apps = ['C:\Apps\KeePass.exe', 'keepass.exe', ' ', 'Code.exe', 'D:/Tools/code.EXE']
";
        let clipboard = Settings::from_toml(text).unwrap().clipboard;
        assert_eq!(clipboard.ignored_apps, ["KeePass.exe", "Code.exe"]);
    }

    #[test]
    fn history_hotkey_is_canonical_and_differs_from_the_tex_hotkey() {
        let parse = |text: &str| Settings::from_toml(text).unwrap();
        let settings = parse("[clipboard]\nhotkey = \"Shift+CTRL+H\"\n");
        assert_eq!(settings.clipboard.hotkey, "ctrl+shift+h");
        let settings = parse("[clipboard]\nhotkey = \"nope\"\n");
        assert_eq!(settings.clipboard.hotkey, "alt+v");
        let settings =
            parse("[general]\nhotkey = \"ctrl+shift+k\"\n[clipboard]\nhotkey = \"shift+ctrl+k\"\n");
        assert_eq!(settings.general.hotkey, "ctrl+shift+k");
        assert_eq!(settings.clipboard.hotkey, "alt+v");
        let settings = parse("[general]\nhotkey = \"alt+v\"\n");
        assert_eq!(settings.general.hotkey, "alt+v");
        assert_eq!(settings.clipboard.hotkey, "ctrl+shift+v");
    }

    #[test]
    fn text_and_images_off_turns_both_back_on() {
        let text = "[clipboard]\ncapture_text = false\ncapture_images = false\n";
        let clipboard = Settings::from_toml(text).unwrap().clipboard;
        assert!(clipboard.capture_text && clipboard.capture_images);
        let text = "[clipboard]\ncapture_text = false\n";
        let clipboard = Settings::from_toml(text).unwrap().clipboard;
        assert!(!clipboard.capture_text && clipboard.capture_images);
    }

    #[test]
    fn converts_to_retention_and_capture_settings() {
        let mut clipboard = Clipboard {
            cleanup_days: 3,
            cleanup_limit_mb: 2048,
            max_item_mb: 25,
            paused: true,
            capture_images: false,
            respect_exclusion: false,
            ignored_apps: vec!["Code.exe".into()],
            ..Clipboard::default()
        };
        assert_eq!(clipboard.retention(), Retention::Time { days: 3 });
        clipboard.cleanup = Cleanup::Storage;
        assert_eq!(
            clipboard.retention(),
            Retention::Storage {
                limit_bytes: 2048 * 1024 * 1024
            }
        );
        let capture = clipboard.capture_settings();
        assert_eq!(capture.max_item_bytes, 25 * 1024 * 1024);
        assert!(capture.paused && capture.capture_text && !capture.capture_images);
        assert!(!capture.respect_exclusion);
        assert_eq!(capture.ignored_apps, ["Code.exe"]);
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
