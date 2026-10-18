use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use slint::language::ColorScheme;
use tex_platform::{
    Accelerator, HotkeySlot, HotkeySupport, Hotkeys, WinKeyHook, WindowRef, system_uses_dark_theme,
    user_language,
};

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
    /// The popup got the focus since it was last shown. Wayland reports a focus loss before
    /// the first focus, which must not close it.
    pub popup_focused: bool,
    /// The same for the history window.
    pub history_focused: bool,
    pub title_pressed_at: Option<Instant>,
    pub theme_preview: Option<Theme>,
    /// The keys the portal bound for each shortcut, as the desktop describes them.
    pub system_keys: [Option<String>; 2],
    /// What each shortcut does, in the user's language, for the portal to show.
    pub hotkey_labels: [String; 2],
    hotkeys: Hotkeys,
    registered: [bool; 2],
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

/// Keycaps for a shortcut: the keys the desktop chose when it decides, else the saved shortcut.
pub fn shortcut_keys(
    support: HotkeySupport,
    system: Option<&str>,
    shortcut: &Shortcut,
) -> Vec<String> {
    match (support, system) {
        (HotkeySupport::System, Some(keys)) if !keys.trim().is_empty() => keys
            .split('+')
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
            .collect(),
        _ => shortcut
            .display_keys()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

fn slot_index(slot: HotkeySlot) -> usize {
    match slot {
        HotkeySlot::Tex => 0,
        HotkeySlot::History => 1,
    }
}

impl AppState {
    pub fn new(settings: Settings, path: PathBuf, hotkeys: Hotkeys) -> Self {
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
            popup_focused: false,
            history_focused: false,
            title_pressed_at: None,
            theme_preview: None,
            system_keys: [None, None],
            hotkey_labels: [String::new(), String::new()],
            hotkeys,
            registered: [false, false],
            winkey: None,
        }
    }

    pub fn shortcut(&self) -> Shortcut {
        Shortcut::parse(&self.settings.general.hotkey)
            .or_else(|| Shortcut::parse(settings::DEFAULT_HOTKEY))
            .expect("default hotkey parses")
    }

    /// The portal binds only after the onboarding, so its dialog does not cover the wizard.
    fn hotkeys_deferred(&self) -> bool {
        self.hotkeys.support() == HotkeySupport::System && !self.settings.general.onboarding_done
    }

    fn accelerator(&self, slot: HotkeySlot, shortcut: &Shortcut) -> Accelerator {
        let mut accelerator = shortcut.accelerator();
        accelerator.label = self.hotkey_labels[slot_index(slot)].clone();
        accelerator
    }

    fn register(&mut self, slot: HotkeySlot, shortcut: &Shortcut) -> bool {
        let accelerator = self.accelerator(slot, shortcut);
        let registered = self.hotkeys.register(slot, &accelerator).is_ok();
        self.registered[slot_index(slot)] = registered;
        registered
    }

    fn unregister(&mut self, slot: HotkeySlot) {
        if std::mem::take(&mut self.registered[slot_index(slot)]) {
            self.hotkeys.unregister(slot);
        }
    }

    /// Keycaps for a shortcut as it works now.
    pub fn hotkey_keys(&self, slot: HotkeySlot) -> Vec<String> {
        let shortcut = match slot {
            HotkeySlot::Tex => self.shortcut(),
            HotkeySlot::History => self.history_shortcut(),
        };
        shortcut_keys(
            self.hotkeys.support(),
            self.system_keys[slot_index(slot)].as_deref(),
            &shortcut,
        )
    }

    pub fn hotkey_text(&self, slot: HotkeySlot) -> String {
        self.hotkey_keys(slot).join("+")
    }

    /// Takes what the portal bound: a shortcut that was asked for and came back without keys
    /// is not active.
    pub fn hotkeys_bound(&mut self, tex: Option<String>, history: Option<String>) {
        self.hotkey_failed = self.registered[0] && tex.is_none();
        self.history_hotkey_failed = self.registered[1] && history.is_none();
        self.system_keys = [tex, history];
    }

    /// Registers the saved shortcut again if it is not registered.
    pub fn resume_hotkey(&mut self) {
        if self.registered[0] || self.hotkeys_deferred() {
            return;
        }
        let shortcut = self.shortcut();
        self.hotkey_failed = !self.register(HotkeySlot::Tex, &shortcut);
    }

    pub fn suspend_hotkey(&mut self) {
        self.unregister(HotkeySlot::Tex);
    }

    /// Swaps the global shortcut. On failure the previous one is registered again.
    pub fn apply_hotkey(&mut self, shortcut: &Shortcut) -> Result<(), HotkeyError> {
        self.suspend_hotkey();
        if shortcut.canonical() == self.history_shortcut().canonical() {
            self.resume_hotkey();
            return Err(HotkeyError::Conflict);
        }
        if self.hotkeys_deferred() || self.register(HotkeySlot::Tex, shortcut) {
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
        if self.registered[1] || self.hotkeys_deferred() {
            return;
        }
        let shortcut = self.history_shortcut();
        self.history_hotkey_failed = !self.register(HotkeySlot::History, &shortcut);
    }

    pub fn suspend_history_hotkey(&mut self) {
        self.unregister(HotkeySlot::History);
    }

    /// Swaps the clipboard history shortcut. On failure the previous one is registered again.
    pub fn apply_history_hotkey(&mut self, shortcut: &Shortcut) -> Result<(), HotkeyError> {
        self.suspend_history_hotkey();
        if shortcut.canonical() == self.shortcut().canonical() {
            self.resume_history_hotkey();
            return Err(HotkeyError::Conflict);
        }
        if self.hotkeys_deferred() || self.register(HotkeySlot::History, shortcut) {
            self.history_hotkey_failed = false;
            self.settings.clipboard.hotkey = shortcut.canonical();
            Ok(())
        } else {
            self.resume_history_hotkey();
            Err(HotkeyError::Register)
        }
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

    fn test_app() -> AppState {
        let mut settings = Settings::default();
        settings.general.hotkey = "ctrl+alt+shift+f23".into();
        settings.clipboard.hotkey = "ctrl+alt+shift+f24".into();
        let hotkeys = Hotkeys::start(|_| {}).unwrap();
        AppState::new(settings, PathBuf::new(), hotkeys)
    }

    #[test]
    fn the_two_shortcuts_cannot_be_the_same() {
        let mut app = test_app();
        let history = app.history_shortcut();
        assert_eq!(app.apply_hotkey(&history), Err(HotkeyError::Conflict));
        assert_eq!(app.settings.general.hotkey, "ctrl+alt+shift+f23");
        let tex = app.shortcut();
        assert_eq!(app.apply_history_hotkey(&tex), Err(HotkeyError::Conflict));
        assert_eq!(app.settings.clipboard.hotkey, "ctrl+alt+shift+f24");
        app.suspend_hotkey();
        app.suspend_history_hotkey();
    }

    #[test]
    fn keycaps_come_from_the_desktop_only_when_it_decides() {
        let shortcut = Shortcut::parse("alt+space").unwrap();
        assert_eq!(
            shortcut_keys(HotkeySupport::System, Some("Meta + Space"), &shortcut),
            ["Meta", "Space"]
        );
        assert_eq!(
            shortcut_keys(HotkeySupport::System, None, &shortcut),
            ["Alt", "Space"]
        );
        assert_eq!(
            shortcut_keys(HotkeySupport::System, Some(" "), &shortcut),
            ["Alt", "Space"]
        );
        assert_eq!(
            shortcut_keys(HotkeySupport::Free, Some("Meta+Space"), &shortcut),
            ["Alt", "Space"]
        );
    }

    #[test]
    fn a_shortcut_the_portal_left_out_is_not_active() {
        let mut app = test_app();
        app.registered = [true, true];
        app.hotkeys_bound(Some("Alt+Space".into()), None);
        assert!(!app.hotkey_failed);
        assert!(app.history_hotkey_failed);
        assert_eq!(app.system_keys[0].as_deref(), Some("Alt+Space"));
        app.registered = [false, false];
        app.hotkeys_bound(None, None);
        assert!(!app.hotkey_failed && !app.history_hotkey_failed);
    }
}
