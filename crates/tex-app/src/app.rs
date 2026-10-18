use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use slint::language::ColorScheme;
use tex_platform::{WinKeyHook, WindowRef, system_uses_dark_theme, user_language};

use crate::clipboard_client::ClientSettings;
use crate::search_client::SearchOptions;
use crate::settings::{self, Language, Search, Settings, Theme};
use crate::shortcut::Shortcut;

/// Everything the UI thread changes at run time: the settings and what applies them.
pub struct AppState {
    pub settings: Settings,
    pub path: PathBuf,
    pub closed_at: Option<Instant>,
    pub hotkey_failed: bool,
    pub history_hotkey_failed: bool,
    pub winkey_error: String,
    pub capture_error: String,
    pub paste_target: WindowRef,
    pub previous_foreground: WindowRef,
    pub title_pressed_at: Option<Instant>,
    pub theme_preview: Option<Theme>,
    hotkeys: GlobalHotKeyManager,
    hotkey: Option<HotKey>,
    history_hotkey: Option<HotKey>,
    winkey: Option<WinKeyHook>,
}

const TRAY_CLICK_GRACE: Duration = Duration::from_millis(300);
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyError {
    Conflict,
    Register,
}

thread_local! {
    static APP: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Clicking the tray icon takes the focus from the popup, which hides it just before the
/// click arrives, so a popup that closed this recently still counts as open.
pub fn tray_click_closes(closed_at: Option<Instant>, now: Instant) -> bool {
    closed_at.is_some_and(|closed_at| now.saturating_duration_since(closed_at) < TRAY_CLICK_GRACE)
}

/// Whether a press at `now` follows the previous one closely enough to be a double click.
pub fn is_double_click(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_some_and(|previous| now.saturating_duration_since(previous) <= DOUBLE_CLICK)
}

pub fn init(state: AppState) {
    APP.with_borrow_mut(|app| *app = Some(state));
}

/// Runs `f` on the app state. Keep it short: never call into Slint callbacks from inside.
pub fn with_app<R>(f: impl FnOnce(&mut AppState) -> R) -> R {
    APP.with_borrow_mut(|app| f(app.as_mut().expect("app state initialized")))
}

impl AppState {
    pub fn new(settings: Settings, path: PathBuf, hotkeys: GlobalHotKeyManager) -> Self {
        Self {
            settings,
            path,
            closed_at: None,
            hotkey_failed: false,
            history_hotkey_failed: false,
            winkey_error: String::new(),
            capture_error: String::new(),
            paste_target: WindowRef::default(),
            previous_foreground: WindowRef::default(),
            title_pressed_at: None,
            theme_preview: None,
            hotkeys,
            hotkey: None,
            history_hotkey: None,
            winkey: None,
        }
    }

    pub fn shortcut(&self) -> Shortcut {
        Shortcut::parse(&self.settings.general.hotkey)
            .or_else(|| Shortcut::parse(settings::DEFAULT_HOTKEY))
            .expect("default hotkey parses")
    }

    /// Registers the saved shortcut again if it is not registered.
    pub fn resume_hotkey(&mut self) {
        if self.hotkey.is_some() {
            return;
        }
        let hotkey = self.shortcut().hotkey();
        self.hotkey_failed = self.hotkeys.register(hotkey).is_err();
        if !self.hotkey_failed {
            self.hotkey = Some(hotkey);
        }
    }

    pub fn suspend_hotkey(&mut self) {
        if let Some(hotkey) = self.hotkey.take() {
            let _ = self.hotkeys.unregister(hotkey);
        }
    }

    /// Swaps the global shortcut. On failure the previous one is registered again.
    pub fn apply_hotkey(&mut self, shortcut: &Shortcut) -> Result<(), HotkeyError> {
        self.suspend_hotkey();
        if shortcut.canonical() == self.history_shortcut().canonical() {
            self.resume_hotkey();
            return Err(HotkeyError::Conflict);
        }
        let hotkey = shortcut.hotkey();
        if self.hotkeys.register(hotkey).is_ok() {
            self.hotkey = Some(hotkey);
            self.hotkey_failed = false;
            self.settings.general.hotkey = shortcut.canonical();
            Ok(())
        } else {
            self.resume_hotkey();
            Err(HotkeyError::Register)
        }
    }

    pub fn history_shortcut(&self) -> Shortcut {
        Shortcut::parse(&self.settings.clipboard.hotkey)
            .or_else(|| Shortcut::parse(settings::DEFAULT_HISTORY_HOTKEY))
            .expect("default history hotkey parses")
    }

    /// Registers the saved clipboard history shortcut again if it is not registered.
    pub fn resume_history_hotkey(&mut self) {
        if self.history_hotkey.is_some() {
            return;
        }
        let hotkey = self.history_shortcut().hotkey();
        self.history_hotkey_failed = self.hotkeys.register(hotkey).is_err();
        if !self.history_hotkey_failed {
            self.history_hotkey = Some(hotkey);
        }
    }

    pub fn suspend_history_hotkey(&mut self) {
        if let Some(hotkey) = self.history_hotkey.take() {
            let _ = self.hotkeys.unregister(hotkey);
        }
    }

    /// Swaps the clipboard history shortcut. On failure the previous one is registered again.
    pub fn apply_history_hotkey(&mut self, shortcut: &Shortcut) -> Result<(), HotkeyError> {
        self.suspend_history_hotkey();
        if shortcut.canonical() == self.shortcut().canonical() {
            self.resume_history_hotkey();
            return Err(HotkeyError::Conflict);
        }
        let hotkey = shortcut.hotkey();
        if self.hotkeys.register(hotkey).is_ok() {
            self.history_hotkey = Some(hotkey);
            self.history_hotkey_failed = false;
            self.settings.clipboard.hotkey = shortcut.canonical();
            Ok(())
        } else {
            self.resume_history_hotkey();
            Err(HotkeyError::Register)
        }
    }

    pub fn is_history_hotkey(&self, id: u32) -> bool {
        self.history_hotkey.is_some_and(|hotkey| hotkey.id() == id)
    }

    pub fn set_winkey(
        &mut self,
        on: bool,
        on_press: impl Fn() + Send + 'static,
    ) -> Result<(), String> {
        self.winkey = None;
        if !on {
            self.winkey_error.clear();
            return Ok(());
        }
        match WinKeyHook::start(on_press) {
            Ok(hook) => {
                self.winkey = Some(hook);
                self.winkey_error.clear();
                Ok(())
            }
            Err(error) => {
                self.winkey_error = error.to_string();
                Err(self.winkey_error.clone())
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        settings::save(&self.path, &self.settings)
    }
}

/// Bundled translation for a language setting: "pt_BR", or "" for the English source text.
pub fn translation(language: Language) -> &'static str {
    match language {
        Language::PtBr => "pt_BR",
        Language::En => "",
        Language::System if user_language().to_ascii_lowercase().starts_with("pt") => "pt_BR",
        Language::System => "",
    }
}

pub fn apply_language(language: Language) {
    let _ = slint::select_bundled_translation(translation(language));
}

pub fn digit_separator(language: Language) -> char {
    if translation(language) == "pt_BR" {
        '.'
    } else {
        ','
    }
}

pub fn decimal_separator(language: Language) -> char {
    if translation(language) == "pt_BR" {
        ','
    } else {
        '.'
    }
}

pub fn color_scheme(theme: Theme) -> ColorScheme {
    match theme {
        Theme::Light => ColorScheme::Light,
        Theme::Dark => ColorScheme::Dark,
        Theme::System if system_uses_dark_theme() => ColorScheme::Dark,
        Theme::System => ColorScheme::Light,
    }
}

pub fn search_options(search: &Search) -> SearchOptions {
    SearchOptions {
        include_hidden: search.include_hidden,
        include_system: search.include_system,
        excluded_folders: search.excluded_folders.clone(),
        show_apps: search.show_apps,
        show_files: search.show_files,
        show_folders: search.show_folders,
    }
}

/// Whether copies are kept: until the onboarding is done the capture stays paused, since one
/// of its questions is whether Tex may keep what is copied.
pub fn capture_paused(settings: &Settings) -> bool {
    settings.clipboard.paused || !settings.general.onboarding_done
}

/// The worker's settings, paused as [`capture_paused`] says.
pub fn client_settings(settings: &Settings) -> ClientSettings {
    let clipboard = &settings.clipboard;
    let mut capture = clipboard.capture_settings();
    capture.paused = capture_paused(settings);
    ClientSettings {
        capture,
        retention: clipboard.retention(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_captured_before_the_onboarding_is_done() {
        let mut settings = Settings::default();
        assert!(client_settings(&settings).capture.paused);
        settings.general.onboarding_done = true;
        assert!(!client_settings(&settings).capture.paused);
        settings.clipboard.paused = true;
        assert!(client_settings(&settings).capture.paused);
        assert_eq!(
            client_settings(&settings).retention,
            settings.clipboard.retention()
        );
    }

    #[test]
    fn a_tray_click_right_after_the_popup_closed_keeps_it_closed() {
        let now = Instant::now();
        assert!(!tray_click_closes(None, now));
        assert!(tray_click_closes(
            Some(now),
            now + Duration::from_millis(100)
        ));
        assert!(!tray_click_closes(Some(now), now + Duration::from_secs(1)));
    }

    #[test]
    fn second_press_within_half_a_second_is_a_double_click() {
        let start = Instant::now();
        assert!(!is_double_click(None, start));
        assert!(is_double_click(
            Some(start),
            start + Duration::from_millis(400)
        ));
        assert!(!is_double_click(
            Some(start),
            start + Duration::from_millis(600)
        ));
    }

    #[test]
    fn the_two_shortcuts_cannot_be_the_same() {
        let mut settings = Settings::default();
        settings.general.hotkey = "ctrl+alt+shift+f23".into();
        settings.clipboard.hotkey = "ctrl+alt+shift+f24".into();
        let hotkeys = GlobalHotKeyManager::new().unwrap();
        let mut app = AppState::new(settings, PathBuf::new(), hotkeys);
        let history = app.history_shortcut();
        assert_eq!(app.apply_hotkey(&history), Err(HotkeyError::Conflict));
        assert_eq!(app.settings.general.hotkey, "ctrl+alt+shift+f23");
        let tex = app.shortcut();
        assert_eq!(app.apply_history_hotkey(&tex), Err(HotkeyError::Conflict));
        assert_eq!(app.settings.clipboard.hotkey, "ctrl+alt+shift+f24");
        assert!(app.is_history_hotkey(history.hotkey().id()) || app.history_hotkey_failed);
        assert!(!app.is_history_hotkey(tex.hotkey().id()));
        app.suspend_hotkey();
        app.suspend_history_hotkey();
    }
}
