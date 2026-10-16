use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Instant;

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use slint::language::ColorScheme;
use tex_platform::{WinKeyHook, system_uses_dark_theme, user_language};

use crate::search_client::SearchOptions;
use crate::settings::{self, Language, Search, Settings, Theme};
use crate::shortcut::Shortcut;

/// Everything the UI thread changes at run time: the settings and what applies them.
pub struct AppState {
    pub settings: Settings,
    pub path: PathBuf,
    pub closed_at: Option<Instant>,
    pub hotkey_failed: bool,
    pub winkey_error: String,
    hotkeys: GlobalHotKeyManager,
    hotkey: Option<HotKey>,
    winkey: Option<WinKeyHook>,
}

thread_local! {
    static APP: RefCell<Option<AppState>> = const { RefCell::new(None) };
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
            winkey_error: String::new(),
            hotkeys,
            hotkey: None,
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

    /// Swaps the global shortcut. On failure the previous one is registered again and `false` returned.
    pub fn apply_hotkey(&mut self, shortcut: &Shortcut) -> bool {
        self.suspend_hotkey();
        let hotkey = shortcut.hotkey();
        if self.hotkeys.register(hotkey).is_ok() {
            self.hotkey = Some(hotkey);
            self.hotkey_failed = false;
            self.settings.general.hotkey = shortcut.canonical();
            true
        } else {
            self.resume_hotkey();
            false
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
