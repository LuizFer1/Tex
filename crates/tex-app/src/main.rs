#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clipboard_client;
mod frame;
mod history;
mod icons;
mod onboarding;
mod placement;
mod reader;
mod reader_client;
mod results;
mod search_client;
mod settings;
mod shortcut;
mod theme;
mod tools;
mod update;
mod update_client;

use std::cell::RefCell;
use std::collections::HashSet;
use std::error::Error;
use std::io;
use std::rc::Rc;
use std::time::Instant;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::language::ColorScheme;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, PhysicalPosition, Rgba8Pixel,
    SharedPixelBuffer, SharedString, StandardListViewItem, VecModel, Weak,
};
use tex_clipboard::{
    Content, EditError, HistoryFilter, Kind, Preview, Rgba, Summary, decode_png, rgba_to_dib,
};
use tex_ipc::{Filter, VolumeState, VolumeStatus};
use tex_platform::{
    ClipboardWatcher, IconPixels, LocalTime, SingleInstance, catch_windows_key, foreground_window,
    local_time, lock_foreground, open_path, paste_into, pick_file, pick_folder, primary_work_area,
    restore_foreground, reveal_in_folder, round_corners, shape_tabbed_window, take_foreground,
    work_area_under_cursor, write_image, write_text,
};

use crate::app::{AppState, HotkeyError, with_app};
use crate::clipboard_client::{
    ClipboardClient, Command as ClipboardCommand, Event as ClipboardEvent, PAGE_SIZE, SEARCH_LIMIT,
    history_folder, now_ms,
};
use crate::history::{HistoryList, day_starts_through, size_parts, when_parts};
use crate::icons::{IconCache, IconLoader, IconState};
use crate::onboarding::{Onboarding, Outcome};
use crate::results::{
    Row, build_rows, file_icon, group_digits, header_counts, readable, ready_summary, short_path,
    status_parts, step_selection,
};
use crate::search_client::{Command, Event, Latest, SearchClient};
use crate::settings::{
    CLEANUP_DAYS, Cleanup, EnterAction, ITEM_SIZES_MB, KeepSearch, Language, OnPick, PopupMonitor,
    STORAGE_LIMITS_MB, Settings, Theme, settings_path,
};
use crate::shortcut::Shortcut;
use crate::theme::{Art, Design};
use crate::update_client::{UpdateClient, UpdateStatus, is_stale};

slint::include_modules!();

const ICON_CACHE_SIZE: usize = 256;
const SEARCH_TAB: i32 = 1;
const CLIPBOARD_TAB: i32 = 2;
const PREVIEW_CHARS: usize = 100_000;

thread_local! {
    static ICONS: RefCell<IconCache<Image>> = RefCell::new(IconCache::new(ICON_CACHE_SIZE));
    static HISTORY: RefCell<Option<HistoryState>> = const { RefCell::new(None) };
    static ONBOARDING: RefCell<Option<Onboarding>> = const { RefCell::new(None) };
    static UPDATES: RefCell<Option<Updates>> = const { RefCell::new(None) };
}

/// Weak handles to every window, cheap to clone into callbacks and across threads.
#[derive(Clone)]
struct Windows {
    popup: Weak<Popup>,
    about: Weak<AboutWindow>,
    settings: Weak<SettingsWindow>,
    history: Weak<HistoryWindow>,
    tray: Weak<TexTray>,
    onboarding: Weak<OnboardingWindow>,
}

/// The history window's rows and what it waits for from the clipboard worker.
struct HistoryState {
    client: ClipboardClient,
    list: HistoryList,
    rows: Rc<VecModel<HistoryRow>>,
    search: Option<u64>,
    more: bool,
    loading_more: bool,
    shown: Option<i64>,
    requested: Option<i64>,
    content: Option<(i64, Option<Content>)>,
    pick: Option<(i64, bool)>,
    edit: Option<(i64, String)>,
    unavailable: String,
}

/// The update worker, the last status it reported and when this process last checked.
struct Updates {
    client: UpdateClient,
    status: UpdateStatus,
    checked_at: Option<Instant>,
}

/// Runs `f` on the history state. Never call into Slint callbacks from inside.
fn with_history<R>(f: impl FnOnce(&mut HistoryState) -> R) -> R {
    HISTORY.with_borrow_mut(|history| f(history.as_mut().expect("history state initialized")))
}

/// Runs `f` on the update state. Never call into Slint from inside.
fn with_updates<R>(f: impl FnOnce(&mut Updates) -> R) -> R {
    UPDATES.with_borrow_mut(|updates| f(updates.as_mut().expect("update state initialized")))
}

/// Runs `f` on the onboarding while it is open.
fn with_onboarding<R>(f: impl FnOnce(&mut Onboarding) -> R) -> Option<R> {
    ONBOARDING.with_borrow_mut(|onboarding| onboarding.as_mut().map(f))
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = settings_path();
    let upgrade = path.exists();
    let settings = settings::load(&path);
    if std::env::args().any(|arg| arg == "--autostart") && !settings.general.start_with_windows {
        return Ok(());
    }
    let Some(_instance) = SingleInstance::acquire("Local\\Tex.SingleInstance")? else {
        return Ok(());
    };

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;

    let popup = Popup::new()?;
    let about = AboutWindow::new()?;
    let settings_window = SettingsWindow::new()?;
    let history = HistoryWindow::new()?;
    let tray = TexTray::new()?;
    let onboarding = OnboardingWindow::new()?;
    about.set_version(env!("CARGO_PKG_VERSION").into());
    settings_window.set_version(env!("CARGO_PKG_VERSION").into());
    let windows = Windows {
        popup: popup.as_weak(),
        about: about.as_weak(),
        settings: settings_window.as_weak(),
        history: history.as_weak(),
        tray: tray.as_weak(),
        onboarding: onboarding.as_weak(),
    };

    app::init(AppState::new(settings, path, GlobalHotKeyManager::new()?));
    let language = with_app(|app| app.settings.general.language);
    app::apply_language(language);
    apply_theme(&windows);
    with_app(|app| apply_categories(&popup, &app.settings));

    let clipboard = ClipboardClient::start(
        history_folder(),
        with_app(|app| app::client_settings(&app.settings)),
        {
            let windows = windows.clone();
            move |event| {
                let windows = windows.clone();
                let _ = slint::invoke_from_event_loop(move || clipboard_event(&windows, event));
            }
        },
    );
    let rows = Rc::new(VecModel::default());
    history.set_rows(ModelRc::from(rows.clone()));
    HISTORY.with_borrow_mut(|state| {
        *state = Some(HistoryState {
            client: clipboard.clone(),
            list: HistoryList::default(),
            rows,
            search: None,
            more: false,
            loading_more: false,
            shown: None,
            requested: None,
            content: None,
            pick: None,
            edit: None,
            unavailable: String::new(),
        });
    });
    let _watcher = match ClipboardWatcher::start(
        {
            let clipboard = clipboard.clone();
            move |formats| clipboard.wanted(formats)
        },
        {
            let clipboard = clipboard.clone();
            move |clip| clipboard.send(ClipboardCommand::Captured(clip))
        },
    ) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            with_app(|app| app.capture_error = error.to_string());
            None
        }
    };
    tray.set_paused(with_app(|app| app::capture_paused(&app.settings)));

    let icons = IconLoader::start({
        let popup = popup.as_weak();
        move |path, pixels| {
            let _ = popup.upgrade_in_event_loop(move |popup| icon_loaded(&popup, path, pixels));
        }
    });

    let latest = Latest::default();
    let client = SearchClient::start(
        std::env::var("USERPROFILE").unwrap_or_default(),
        latest.clone(),
        {
            let windows = windows.clone();
            let icons = icons.clone();
            move |event| {
                let windows = windows.clone();
                let icons = icons.clone();
                let latest = latest.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    show_event(&windows, event, &icons, &latest)
                });
            }
        },
    );

    with_app(|app| {
        app.resume_hotkey();
        app.resume_history_hotkey();
    });
    GlobalHotKeyEvent::set_event_handler(Some({
        let windows = windows.clone();
        let client = client.clone();
        move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let windows = windows.clone();
            let client = client.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if with_app(|app| app.is_history_hotkey(event.id)) {
                    toggle_history(&windows);
                } else {
                    toggle_popup(&windows, &client);
                }
            });
        }
    }));
    if with_app(|app| app.settings.winkey_active()) {
        let on_press = winkey_handler(&windows, &client);
        if with_app(|app| app.set_winkey(true, on_press)).is_err() {
            with_app(|app| app.settings.general.windows_key = false);
        }
    }

    wire_popup(&popup, &windows, &client);
    wire_settings(&settings_window, &windows, &client);
    wire_history(&history, &windows, &client);
    wire_onboarding(&onboarding, &windows, &client);
    for window in [
        about.window(),
        settings_window.window(),
        onboarding.window(),
    ] {
        let windows = windows.clone();
        window.on_winit_window_event(move |window, event| {
            if matches!(event, WindowEvent::ThemeChanged(_)) {
                theme_changed(&windows);
            }
            if matches!(event, WindowEvent::Resized(_))
                && let Some(settings) = windows.settings.upgrade()
                && std::ptr::eq(settings.window(), window)
            {
                let maximized = window
                    .with_winit_window(|w| w.is_maximized())
                    .unwrap_or(false);
                settings.set_window_maximized(maximized);
            }
            EventResult::Propagate
        });
    }

    let updates = UpdateClient::start(history_folder().join("updates"), {
        let windows = windows.clone();
        move |status| {
            let windows = windows.clone();
            let _ = slint::invoke_from_event_loop(move || update_status(&windows, status));
        }
    });
    UPDATES.with_borrow_mut(|state| {
        *state = Some(Updates {
            client: updates,
            status: UpdateStatus::Idle,
            checked_at: None,
        });
    });
    if with_app(|app| app.settings.general.auto_update) {
        check_updates(&windows);
    }

    tray.on_open({
        let windows = windows.clone();
        let client = client.clone();
        move || show_popup(&windows, &client)
    });
    tray.on_toggle({
        let windows = windows.clone();
        let client = client.clone();
        move || tray_toggle(&windows, &client)
    });
    tray.on_history({
        let windows = windows.clone();
        move || show_history(&windows)
    });
    tray.on_settings({
        let windows = windows.clone();
        let client = client.clone();
        move || show_settings(&windows, &client, None)
    });
    tray.on_about({
        let windows = windows.clone();
        move || {
            apply_theme(&windows);
            if let Some(about) = windows.about.upgrade() {
                let _ = about.show();
            }
        }
    });
    tray.on_toggle_pause({
        let windows = windows.clone();
        move || toggle_pause(&windows)
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.on_install_update({
        let windows = windows.clone();
        move || install_update(&windows)
    });
    reader_client::reload_catalog();
    tray.show()?;
    if !with_app(|app| app.settings.general.onboarding_done) {
        start_onboarding(&windows, upgrade);
    }

    #[cfg(debug_assertions)]
    if let Some(path) = std::env::args().skip_while(|arg| arg != "--read").nth(1) {
        reader_client::open(&windows, std::path::Path::new(&path));
    }

    slint::run_event_loop_until_quit()?;
    Ok(())
}

/// Flips the pause the tray shows; until the onboarding is done that is always "paused", so it saves "resumed".
fn toggle_pause(windows: &Windows) {
    let (old, new) = with_app(|app| {
        let old = app.settings.clone();
        app.settings.clipboard.paused = !app::capture_paused(&old);
        (old, app.settings.clone())
    });
    if let Some(window) = windows.settings.upgrade() {
        save_settings(&window);
    }
    clipboard_changed(windows);
    sync_onboarding(windows, &old, &new);
}

/// Starts a check unless one is running or the onboarding has not finished.
fn check_updates(windows: &Windows) {
    if !with_app(|app| app.settings.general.onboarding_done) {
        return;
    }
    let status = with_updates(|updates| {
        (!updates.status.busy()).then(|| {
            updates.checked_at = Some(Instant::now());
            updates.status = UpdateStatus::Checking;
            updates.client.check();
            updates.status.clone()
        })
    });
    if let Some(status) = status {
        show_update_status(windows, &status);
    }
}

fn check_updates_if_stale(windows: &Windows) {
    let auto = with_app(|app| app.settings.general.auto_update);
    if auto && with_updates(|updates| is_stale(updates.checked_at, Instant::now())) {
        check_updates(windows);
    }
}

fn install_update(windows: &Windows) {
    let status = with_updates(|updates| {
        let version = updates.status.ready_version()?.to_owned();
        updates.status = UpdateStatus::Installing(version);
        updates.client.apply();
        Some(updates.status.clone())
    });
    if let Some(status) = status {
        show_update_status(windows, &status);
    }
}

fn update_status(windows: &Windows, status: UpdateStatus) {
    with_updates(|updates| updates.status = status.clone());
    show_update_status(windows, &status);
}

fn show_update_status(windows: &Windows, status: &UpdateStatus) {
    let (state, version) = status.parts();
    if let Some(tray) = windows.tray.upgrade() {
        tray.set_update_version(status.ready_version().unwrap_or_default().into());
    }
    if let Some(window) = windows.settings.upgrade() {
        window.set_update_state(state);
        window.set_update_version(version.into());
    }
}

fn open_from_key(windows: &Windows, client: &SearchClient) {
    let windows = windows.clone();
    let client = client.clone();
    let _ = slint::invoke_from_event_loop(move || toggle_popup(&windows, &client));
}

fn winkey_handler(windows: &Windows, client: &SearchClient) -> impl Fn() + Send + 'static {
    let windows = windows.clone();
    let client = client.clone();
    move || open_from_key(&windows, &client)
}

fn wire_popup(popup: &Popup, windows: &Windows, client: &SearchClient) {
    popup.on_dismissed({
        let popup = popup.as_weak();
        move || {
            if let Some(popup) = popup.upgrade() {
                close_popup(&popup);
            }
        }
    });
    popup.window().on_winit_window_event({
        let popup = popup.as_weak();
        let windows = windows.clone();
        move |_, event| {
            match event {
                WindowEvent::Focused(false) => {
                    let _ = popup.upgrade_in_event_loop(|popup| hide_popup(&popup));
                }
                WindowEvent::ThemeChanged(_) => theme_changed(&windows),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    let _ = popup.upgrade_in_event_loop(|popup| {
                        shape_window(
                            popup.window(),
                            popup.get_tab_width(),
                            popup.get_tab_height(),
                        );
                    });
                }
                _ => {}
            }
            EventResult::Propagate
        }
    });
    popup.on_query_edited({
        let popup = popup.as_weak();
        let client = client.clone();
        move |_| {
            if let Some(popup) = popup.upgrade() {
                search(&popup, &client);
            }
        }
    });
    popup.on_filter_changed({
        let popup = popup.as_weak();
        let client = client.clone();
        move |_| {
            if let Some(popup) = popup.upgrade() {
                search(&popup, &client);
            }
        }
    });
    popup.on_step({
        let popup = popup.as_weak();
        move |step| {
            if let Some(popup) = popup.upgrade() {
                move_selection(&popup, step);
            }
        }
    });
    popup.on_select({
        let popup = popup.as_weak();
        move |index| {
            if let Some(popup) = popup.upgrade() {
                popup.set_selected(index);
            }
        }
    });
    popup.on_activate({
        let popup = popup.as_weak();
        move |index, alternate| {
            if let Some(popup) = popup.upgrade() {
                activate(&popup, index, alternate);
            }
        }
    });
    popup.on_read({
        let popup = popup.as_weak();
        let windows = windows.clone();
        move |index| {
            let Some(popup) = popup.upgrade() else {
                return;
            };
            let Some(row) = usize::try_from(index)
                .ok()
                .and_then(|index| popup.get_rows().row_data(index))
            else {
                return;
            };
            if row.readable {
                reader_client::open(&windows, std::path::Path::new(row.path.as_str()));
            }
        }
    });
    popup.on_open_settings({
        let windows = windows.clone();
        let client = client.clone();
        move || {
            if let Some(popup) = windows.popup.upgrade() {
                hide_popup(&popup);
            }
            show_settings(&windows, &client, None);
        }
    });
}

fn wire_settings(window: &SettingsWindow, windows: &Windows, client: &SearchClient) {
    frame::wire_frame!(window);
    window.on_edited({
        let windows = windows.clone();
        let client = client.clone();
        move || settings_edited(&windows, &client)
    });
    window.on_check_updates({
        let windows = windows.clone();
        move || check_updates(&windows)
    });
    window.on_install_update({
        let windows = windows.clone();
        move || install_update(&windows)
    });
    window.on_redo_onboarding({
        let windows = windows.clone();
        move || redo_onboarding(&windows)
    });
    window.on_change_hotkey({
        let window = window.as_weak();
        move || {
            suspend_hotkeys();
            if let Some(window) = window.upgrade() {
                window.set_hotkey_error(0);
                window.set_recording(true);
            }
        }
    });
    window.on_cancel_recording({
        let window = window.as_weak();
        move || {
            resume_hotkeys();
            if let Some(window) = window.upgrade() {
                window.set_recording(false);
            }
        }
    });
    window.on_shortcut_captured({
        let window = window.as_weak();
        move |text, ctrl, alt, shift, win| {
            let Some(window) = window.upgrade() else {
                return;
            };
            if let Some(shortcut) = Shortcut::from_slint(&text, ctrl, alt, shift, win) {
                change_hotkey(&window, &shortcut);
            }
        }
    });
    window.on_reset_hotkey({
        let window = window.as_weak();
        move || {
            if let (Some(window), Some(shortcut)) =
                (window.upgrade(), Shortcut::parse(settings::DEFAULT_HOTKEY))
            {
                change_hotkey(&window, &shortcut);
            }
        }
    });
    window.on_add_folder({
        let windows = windows.clone();
        move || {
            let Some(title) = windows
                .settings
                .upgrade()
                .map(|window| window.get_folder_picker_title())
            else {
                return;
            };
            let windows = windows.clone();
            pick_folder(&title, move |folder| {
                let Some(folder) = folder else {
                    return;
                };
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(|app| {
                        app.settings.search.excluded_folders.push(folder);
                        app.settings.normalize();
                    });
                    folders_changed(&windows);
                });
            });
        }
    });
    window.on_remove_folder({
        let windows = windows.clone();
        move |index| {
            with_app(|app| {
                if let Ok(index) = usize::try_from(index)
                    && index < app.settings.search.excluded_folders.len()
                {
                    app.settings.search.excluded_folders.remove(index);
                }
            });
            folders_changed(&windows);
        }
    });
    window.on_refresh_drives({
        let client = client.clone();
        move || client.send(Command::Status)
    });
    wire_clipboard_tab(window, windows);
    window.window().on_close_requested({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade()
                && (window.get_recording() || window.get_history_recording())
            {
                window.set_recording(false);
                window.set_history_recording(false);
                resume_hotkeys();
            }
            CloseRequestResponse::HideWindow
        }
    });
}

fn wire_clipboard_tab(window: &SettingsWindow, windows: &Windows) {
    window.on_change_history_hotkey({
        let window = window.as_weak();
        move || {
            suspend_hotkeys();
            if let Some(window) = window.upgrade() {
                window.set_history_hotkey_error(0);
                window.set_history_recording(true);
            }
        }
    });
    window.on_cancel_history_recording({
        let window = window.as_weak();
        move || {
            resume_hotkeys();
            if let Some(window) = window.upgrade() {
                window.set_history_recording(false);
            }
        }
    });
    window.on_history_shortcut_captured({
        let window = window.as_weak();
        move |text, ctrl, alt, shift, win| {
            let Some(window) = window.upgrade() else {
                return;
            };
            if let Some(shortcut) = Shortcut::from_slint(&text, ctrl, alt, shift, win) {
                change_history_hotkey(&window, &shortcut);
            }
        }
    });
    window.on_reset_history_hotkey({
        let window = window.as_weak();
        move || {
            if let (Some(window), Some(shortcut)) = (
                window.upgrade(),
                Shortcut::parse(settings::DEFAULT_HISTORY_HOTKEY),
            ) {
                change_history_hotkey(&window, &shortcut);
            }
        }
    });
    window.on_add_app({
        let windows = windows.clone();
        move || {
            let Some((title, filter)) = windows.settings.upgrade().map(|window| {
                (
                    window.get_app_picker_title(),
                    window.get_app_picker_filter(),
                )
            }) else {
                return;
            };
            let windows = windows.clone();
            pick_file(&title, &filter, move |path| {
                let Some(path) = path else {
                    return;
                };
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(|app| {
                        app.settings.clipboard.ignored_apps.push(path);
                        app.settings.normalize();
                    });
                    ignored_apps_changed(&windows);
                });
            });
        }
    });
    window.on_remove_app({
        let windows = windows.clone();
        move |index| {
            with_app(|app| {
                if let Ok(index) = usize::try_from(index)
                    && index < app.settings.clipboard.ignored_apps.len()
                {
                    app.settings.clipboard.ignored_apps.remove(index);
                }
            });
            ignored_apps_changed(&windows);
        }
    });
    window.on_clear_history(|| send_history(ClipboardCommand::Clear));
}

/// While a shortcut is recorded neither one is registered, so pressing the other one is
/// recorded (and refused) instead of opening its window.
fn suspend_hotkeys() {
    with_app(|app| {
        app.suspend_hotkey();
        app.suspend_history_hotkey();
    });
}

fn resume_hotkeys() {
    with_app(|app| {
        app.resume_hotkey();
        app.resume_history_hotkey();
    });
}

/// The error shown under a shortcut: when the previous one could not be registered either,
/// nothing was kept and the shortcut is not active.
fn hotkey_error(result: Result<(), HotkeyError>, previous_failed: bool) -> i32 {
    match result {
        Ok(()) => 0,
        Err(HotkeyError::Register) if previous_failed => 3,
        Err(HotkeyError::Register) => 1,
        Err(HotkeyError::Conflict) => 2,
    }
}

fn keys_model(shortcut: &Shortcut) -> ModelRc<SharedString> {
    let keys: Vec<SharedString> = shortcut
        .display_keys()
        .into_iter()
        .map(Into::into)
        .collect();
    ModelRc::new(VecModel::from(keys))
}

fn change_history_hotkey(window: &SettingsWindow, shortcut: &Shortcut) {
    let result = with_app(|app| app.apply_history_hotkey(shortcut));
    resume_hotkeys();
    window.set_history_recording(false);
    window.set_history_hotkey_error(hotkey_error(
        result,
        with_app(|app| app.history_hotkey_failed),
    ));
    window.set_history_hotkey_text(with_app(|app| app.history_shortcut().display()).into());
    window.set_history_hotkey_keys(with_app(|app| keys_model(&app.history_shortcut())));
    if result.is_ok() {
        save_settings(window);
    }
}

fn ignored_apps_changed(windows: &Windows) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    window.set_selected_ignored(-1);
    with_app(|app| fill_ignored_apps(&window, &app.settings));
    save_settings(&window);
    clipboard_changed(windows);
}

/// Sends the [clipboard] settings to the worker and shows the pause state everywhere: the
/// tray and the history show whether copies are kept, the settings window the saved value.
fn clipboard_changed(windows: &Windows) {
    let (settings, paused, paste) = with_app(|app| {
        let clipboard = &app.settings.clipboard;
        (
            app::client_settings(&app.settings),
            clipboard.paused,
            clipboard.on_pick == OnPick::CopyAndPaste,
        )
    });
    let capture_paused = settings.capture.paused;
    with_history(|history| history.client.configure(settings));
    if let Some(tray) = windows.tray.upgrade() {
        tray.set_paused(capture_paused);
    }
    if let Some(history) = windows.history.upgrade() {
        history.set_paused(capture_paused);
        history.set_paste_by_default(paste);
    }
    if let Some(window) = windows.settings.upgrade() {
        window.set_paused(paused);
    }
}

fn change_hotkey(window: &SettingsWindow, shortcut: &Shortcut) {
    let result = with_app(|app| app.apply_hotkey(shortcut));
    resume_hotkeys();
    window.set_recording(false);
    window.set_hotkey_error(hotkey_error(result, with_app(|app| app.hotkey_failed)));
    window.set_hotkey_text(with_app(|app| app.shortcut().display()).into());
    window.set_hotkey_keys(with_app(|app| keys_model(&app.shortcut())));
    if result.is_ok() {
        save_settings(window);
    }
}

fn folders_changed(windows: &Windows) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    window.set_selected_excluded(-1);
    with_app(|app| fill_folders(&window, &app.settings));
    save_settings(&window);
}

fn save_settings(window: &SettingsWindow) {
    let error = with_app(|app| app.save()).err();
    window.set_save_error(
        error
            .map(|error| error.to_string())
            .unwrap_or_default()
            .into(),
    );
}

/// Applies `new` over `old` (Windows-key hook, language, theme, categories, clipboard,
/// update check) and saves. Returns what was applied: the hook may fail and turn the Windows
/// key off.
fn apply_settings(
    windows: &Windows,
    client: &SearchClient,
    old: &Settings,
    mut new: Settings,
) -> Settings {
    if new.winkey_active() != old.winkey_active() {
        let on_press = winkey_handler(windows, client);
        if with_app(|app| app.set_winkey(new.winkey_active(), on_press)).is_err() {
            new.general.windows_key = false;
        }
    }
    with_app(|app| app.settings = new.clone());
    if new.general.language != old.general.language {
        app::apply_language(new.general.language);
    }
    if new.general.theme != old.general.theme {
        apply_theme(windows);
    }
    if let Some(popup) = windows.popup.upgrade() {
        apply_categories(&popup, &new);
    }
    if let Some(window) = windows.settings.upgrade() {
        save_settings(&window);
    }
    if new.clipboard != old.clipboard || new.general.onboarding_done != old.general.onboarding_done
    {
        clipboard_changed(windows);
    }
    if new.general.language != old.general.language {
        send_history(ClipboardCommand::Usage);
    }
    let wants_updates =
        |settings: &Settings| settings.general.onboarding_done && settings.general.auto_update;
    if wants_updates(&new) && !wants_updates(old) {
        check_updates(windows);
    }
    new
}

/// Reads the settings window back, applies whatever changed and saves.
fn settings_edited(windows: &Windows, client: &SearchClient) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    let old = with_app(|app| app.settings.clone());
    let new = read_settings(&window, &old);
    let requested = new.general.windows_key;
    let applied = apply_settings(windows, client, &old, new);
    if requested != old.general.windows_key {
        window.set_windows_key(applied.general.windows_key);
        window.set_winkey_error(with_app(|app| app.winkey_error.clone()).into());
    }
    sync_onboarding(windows, &old, &applied);
}

/// Takes settings changed outside the onboarding into its answers while it is open; a new
/// theme replaces the one being previewed.
fn sync_onboarding(windows: &Windows, old: &Settings, new: &Settings) {
    if with_onboarding(|onboarding| onboarding.sync(old, new)).is_none() {
        return;
    }
    if new.general.theme != old.general.theme {
        with_app(|app| app.theme_preview = None);
        apply_theme(windows);
    }
    refresh_onboarding(windows);
}

fn theme_changed(windows: &Windows) {
    let windows = windows.clone();
    let _ = slint::invoke_from_event_loop(move || apply_theme(&windows));
}

fn rgb(value: u32) -> slint::Color {
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Copies a design into one window's `Tokens` (each window has its own copy of the global).
fn apply_design(tokens: &Tokens<'_>, artwork: &Artwork<'_>, design: &Design, dark: bool) {
    let c = theme::colors(design, dark);
    tokens.set_action(rgb(c.action));
    tokens.set_action_strong(rgb(c.action_strong));
    tokens.set_highlight(rgb(c.highlight));
    tokens.set_paper(rgb(c.paper));
    tokens.set_paper_2(rgb(c.paper_2));
    tokens.set_field(rgb(c.field));
    tokens.set_row_hover(rgb(c.row_hover));
    tokens.set_ink(rgb(c.ink));
    tokens.set_ink_2(rgb(c.ink_2));
    tokens.set_ink_3(rgb(c.ink_3));
    tokens.set_muted(rgb(c.muted));
    tokens.set_muted_2(rgb(c.muted_2));
    tokens.set_border(rgb(c.border));
    tokens.set_border_2(rgb(c.border_2));
    tokens.set_control_off(rgb(c.control_off));
    tokens.set_control_off_2(rgb(c.control_off_2));
    tokens.set_warn(rgb(c.warn));
    tokens.set_warn_text(rgb(c.warn_text));
    tokens.set_danger(rgb(c.danger));
    tokens.set_danger_hover(rgb(c.danger_hover));
    tokens.set_danger_bg(rgb(c.danger_bg));
    tokens.set_on_action(rgb(c.on_action));
    tokens.set_on_action_muted(rgb(c.on_action_muted));
    tokens.set_selection(rgb(c.selection));
    tokens.set_type_file(rgb(c.type_file));
    tokens.set_type_folder(rgb(c.type_folder));
    tokens.set_type_image(rgb(c.type_image));
    tokens.set_type_app(rgb(c.type_app));
    tokens.set_scrollbar(rgb(c.scrollbar));
    tokens.set_banner(rgb(c.banner));
    tokens.set_banner_ink(rgb(c.banner_ink));
    tokens.set_banner_ink_2(rgb(c.banner_ink_2));
    tokens.set_banner_kicker(rgb(c.banner_kicker));
    tokens.set_banner_hover(rgb(c.banner_hover));
    tokens.set_close_hover(rgb(c.close_hover));
    tokens.set_close_hover_ink(rgb(c.close_hover_ink));
    tokens.set_action_tint(rgb(c.action_tint));
    tokens.set_window_ring(rgb(c.window_ring));
    let shadow = rgb(c.shadow);
    tokens.set_shadow(slint::Color::from_argb_u8(
        46,
        shadow.red(),
        shadow.green(),
        shadow.blue(),
    ));
    let (light, dark_colors) = (&design.light, &design.dark);
    tokens.set_preview_light_box(rgb(light.paper));
    tokens.set_preview_light_ink(rgb(light.ink));
    tokens.set_preview_light_line(rgb(light.border));
    tokens.set_preview_light_background(rgb(light.paper_2));
    tokens.set_preview_dark_box(rgb(dark_colors.paper));
    tokens.set_preview_dark_ink(rgb(dark_colors.ink));
    tokens.set_preview_dark_line(rgb(dark_colors.border));
    tokens.set_preview_dark_background(rgb(dark_colors.paper_2));

    let s = &design.shape;
    tokens.set_radius_control(s.radius_control);
    tokens.set_radius_card(s.radius_card);
    tokens.set_radius_tab(s.radius_tab);
    tokens.set_radius_window(s.radius_window);
    tokens.set_radius_art(s.radius_art);
    tokens.set_border_control(s.border_control);
    tokens.set_border_strong(s.border_strong);
    tokens.set_focus_width(s.focus_width);
    tokens.set_scrollbar_width(s.scrollbar_width);

    let t = &design.typography;
    tokens.set_ui_font(t.ui_font.into());
    tokens.set_mono_font(t.mono_font.into());
    tokens.set_size_search(t.size_search);
    tokens.set_size_history_input(t.size_history_input);
    tokens.set_size_title(t.size_title);
    tokens.set_size_banner(t.size_banner);
    tokens.set_size_result(t.size_result);
    tokens.set_size_tab(t.size_tab);
    tokens.set_size_label(t.size_label);
    tokens.set_size_check(t.size_check);
    tokens.set_size_body(t.size_body);
    tokens.set_size_button(t.size_button);
    tokens.set_size_desc(t.size_desc);
    tokens.set_size_path(t.size_path);
    tokens.set_size_meta(t.size_meta);
    tokens.set_size_kicker(t.size_kicker);
    tokens.set_size_key(t.size_key);
    tokens.set_size_hero(t.size_hero);
    tokens.set_size_question(t.size_question);
    tokens.set_size_badge(t.size_badge);

    match design.art {
        Art::Mascot => {
            tokens.set_app_icon(artwork.get_mascot_icon());
            tokens.set_art_search(artwork.get_mascot_search());
            tokens.set_art_clipboard(artwork.get_mascot_clipboard());
            tokens.set_art_banner(artwork.get_mascot_banner());
            tokens.set_art_welcome(artwork.get_mascot_welcome());
            tokens.set_art_done(artwork.get_mascot_done());
        }
    }
}

fn apply_theme(windows: &Windows) {
    let scheme = app::color_scheme(with_app(|app| {
        app.theme_preview.unwrap_or(app.settings.general.theme)
    }));
    let dark = scheme == ColorScheme::Dark;
    let design = theme::design(theme::DEFAULT);
    if let Some(popup) = windows.popup.upgrade() {
        popup.set_scheme(scheme);
        apply_design(
            &popup.global::<Tokens>(),
            &popup.global::<Artwork>(),
            design,
            dark,
        );
    }
    if let Some(about) = windows.about.upgrade() {
        about.set_scheme(scheme);
        apply_design(
            &about.global::<Tokens>(),
            &about.global::<Artwork>(),
            design,
            dark,
        );
    }
    if let Some(settings) = windows.settings.upgrade() {
        settings.set_scheme(scheme);
        apply_design(
            &settings.global::<Tokens>(),
            &settings.global::<Artwork>(),
            design,
            dark,
        );
    }
    if let Some(history) = windows.history.upgrade() {
        history.set_scheme(scheme);
        apply_design(
            &history.global::<Tokens>(),
            &history.global::<Artwork>(),
            design,
            dark,
        );
    }
    if let Some(onboarding) = windows.onboarding.upgrade() {
        apply_design(
            &onboarding.global::<Tokens>(),
            &onboarding.global::<Artwork>(),
            design,
            dark,
        );
    }
    reader_client::each(|reader| {
        reader.set_scheme(scheme);
        apply_design(
            &reader.global::<Tokens>(),
            &reader.global::<Artwork>(),
            design,
            dark,
        );
    });
}

fn apply_categories(popup: &Popup, settings: &Settings) {
    let search = &settings.search;
    popup.set_show_apps(search.show_apps);
    popup.set_show_files(search.show_files);
    popup.set_show_folders(search.show_folders);
    popup.set_enter_reveals(search.enter_action == EnterAction::Reveal);
    let hidden = match popup.get_filter() {
        1 => !search.show_apps,
        2 => !search.show_files,
        3 => !search.show_folders,
        _ => false,
    };
    if hidden {
        popup.set_filter(0);
    }
}

fn fill_settings(window: &SettingsWindow, app: &AppState) {
    let general = &app.settings.general;
    let search = &app.settings.search;
    window.set_language(match general.language {
        Language::System => 0,
        Language::PtBr => 1,
        Language::En => 2,
    });
    window.set_theme(match general.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    window.set_hotkey_text(app.shortcut().display().into());
    window.set_hotkey_keys(keys_model(&app.shortcut()));
    window.set_hotkey_error(if app.hotkey_failed { 3 } else { 0 });
    window.set_recording(false);
    window.set_windows_key(general.windows_key);
    window.set_winkey_error(app.winkey_error.as_str().into());
    window.set_start_with_windows(general.start_with_windows);
    window.set_auto_update(general.auto_update);
    window.set_popup_monitor(match general.popup_monitor {
        PopupMonitor::Cursor => 0,
        PopupMonitor::Primary => 1,
    });
    window.set_keep_search(match search.keep_last_search {
        KeepSearch::Never => 0,
        KeepSearch::Seconds30 => 1,
        KeepSearch::Minute1 => 2,
        KeepSearch::Minutes5 => 3,
        KeepSearch::Always => 4,
    });
    window.set_include_hidden(search.include_hidden);
    window.set_include_system(search.include_system);
    window.set_show_apps(search.show_apps);
    window.set_show_files(search.show_files);
    window.set_show_folders(search.show_folders);
    window.set_enter_action(match search.enter_action {
        EnterAction::Open => 0,
        EnterAction::Reveal => 1,
    });
    window.set_selected_excluded(-1);
    fill_folders(window, &app.settings);
    let clipboard = &app.settings.clipboard;
    window.set_history_hotkey_text(app.history_shortcut().display().into());
    window.set_history_hotkey_keys(keys_model(&app.history_shortcut()));
    window.set_history_hotkey_error(if app.history_hotkey_failed { 3 } else { 0 });
    window.set_history_recording(false);
    window.set_paused(clipboard.paused);
    window.set_capture_text(clipboard.capture_text);
    window.set_capture_images(clipboard.capture_images);
    window.set_item_size(preset_index(&ITEM_SIZES_MB, clipboard.max_item_mb));
    window.set_on_pick(match clipboard.on_pick {
        OnPick::Copy => 0,
        OnPick::CopyAndPaste => 1,
    });
    window.set_cleanup(match clipboard.cleanup {
        Cleanup::Time => 0,
        Cleanup::Storage => 1,
    });
    window.set_cleanup_days(preset_index(&CLEANUP_DAYS, clipboard.cleanup_days));
    window.set_cleanup_limit(preset_index(&STORAGE_LIMITS_MB, clipboard.cleanup_limit_mb));
    window.set_respect_exclusion(clipboard.respect_exclusion);
    window.set_selected_ignored(-1);
    fill_ignored_apps(window, &app.settings);
    window.set_confirm_clear(false);
    window.set_capture_error(app.capture_error.as_str().into());
}

fn preset_index(presets: &[u32], value: u32) -> i32 {
    presets
        .iter()
        .position(|preset| *preset == value)
        .map_or(0, |index| index as i32)
}

fn preset_at(presets: &[u32], index: i32, current: u32) -> u32 {
    usize::try_from(index)
        .ok()
        .and_then(|index| presets.get(index).copied())
        .unwrap_or(current)
}

fn fill_ignored_apps(window: &SettingsWindow, settings: &Settings) {
    let items: Vec<StandardListViewItem> = settings
        .clipboard
        .ignored_apps
        .iter()
        .map(|app| StandardListViewItem::from(app.as_str()))
        .collect();
    window.set_ignored_apps(ModelRc::new(VecModel::from(items)));
}

fn fill_folders(window: &SettingsWindow, settings: &Settings) {
    let items: Vec<StandardListViewItem> = settings
        .search
        .excluded_folders
        .iter()
        .map(|folder| StandardListViewItem::from(folder.as_str()))
        .collect();
    window.set_excluded(ModelRc::new(VecModel::from(items)));
}

/// The settings shown in the window, on top of `current` for what the window does not edit.
fn read_settings(window: &SettingsWindow, current: &Settings) -> Settings {
    let mut settings = current.clone();
    let general = &mut settings.general;
    general.language = match window.get_language() {
        1 => Language::PtBr,
        2 => Language::En,
        _ => Language::System,
    };
    general.theme = match window.get_theme() {
        1 => Theme::Light,
        2 => Theme::Dark,
        _ => Theme::System,
    };
    general.windows_key = window.get_windows_key();
    general.start_with_windows = window.get_start_with_windows();
    general.auto_update = window.get_auto_update();
    general.popup_monitor = match window.get_popup_monitor() {
        1 => PopupMonitor::Primary,
        _ => PopupMonitor::Cursor,
    };
    let search = &mut settings.search;
    search.keep_last_search = match window.get_keep_search() {
        0 => KeepSearch::Never,
        1 => KeepSearch::Seconds30,
        3 => KeepSearch::Minutes5,
        4 => KeepSearch::Always,
        _ => KeepSearch::Minute1,
    };
    search.include_hidden = window.get_include_hidden();
    search.include_system = window.get_include_system();
    search.show_apps = window.get_show_apps();
    search.show_files = window.get_show_files();
    search.show_folders = window.get_show_folders();
    search.enter_action = match window.get_enter_action() {
        1 => EnterAction::Reveal,
        _ => EnterAction::Open,
    };
    let clipboard = &mut settings.clipboard;
    clipboard.paused = window.get_paused();
    clipboard.capture_text = window.get_capture_text();
    clipboard.capture_images = window.get_capture_images();
    clipboard.max_item_mb = preset_at(
        &ITEM_SIZES_MB,
        window.get_item_size(),
        clipboard.max_item_mb,
    );
    clipboard.on_pick = match window.get_on_pick() {
        1 => OnPick::CopyAndPaste,
        _ => OnPick::Copy,
    };
    clipboard.cleanup = match window.get_cleanup() {
        1 => Cleanup::Storage,
        _ => Cleanup::Time,
    };
    clipboard.cleanup_days = preset_at(
        &CLEANUP_DAYS,
        window.get_cleanup_days(),
        clipboard.cleanup_days,
    );
    clipboard.cleanup_limit_mb = preset_at(
        &STORAGE_LIMITS_MB,
        window.get_cleanup_limit(),
        clipboard.cleanup_limit_mb,
    );
    clipboard.respect_exclusion = window.get_respect_exclusion();
    settings.normalize();
    settings
}

/// Shows the settings window, on `tab` when given.
fn show_settings(windows: &Windows, client: &SearchClient, tab: Option<i32>) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    apply_theme(windows);
    with_app(|app| fill_settings(&window, app));
    show_update_status(windows, &with_updates(|updates| updates.status.clone()));
    window.set_history_unavailable(with_history(|history| history.unavailable.clone()).into());
    if let Some(tab) = tab {
        window.set_current_tab(tab);
    }
    if window.get_current_tab() == SEARCH_TAB {
        client.send(Command::Status);
    }
    send_history(ClipboardCommand::Usage);
    let _ = window.show();
    if let Some(hwnd) = window_hwnd(window.window()) {
        round_corners(hwnd);
    }
    window
        .window()
        .with_winit_window(|window| window.focus_window());
}

/// Opens the onboarding centered on the primary monitor, with the current settings selected.
fn start_onboarding(windows: &Windows, upgrade: bool) {
    let onboarding = with_app(|app| Onboarding::new(&app.settings, upgrade));
    ONBOARDING.with_borrow_mut(|state| *state = Some(onboarding));
    let Some(window) = windows.onboarding.upgrade() else {
        return;
    };
    refresh_onboarding(windows);
    apply_theme(windows);
    if let Some(area) = primary_work_area() {
        let (x, y) = placement::centered_origin(
            &area,
            placement::ONBOARDING_WIDTH,
            placement::ONBOARDING_HEIGHT,
        );
        window.window().set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    if let Some(hwnd) = window_hwnd(window.window()) {
        round_corners(hwnd);
    }
    window
        .window()
        .with_winit_window(|window| window.focus_window());
}

/// Hides the settings window and opens the onboarding again with the current settings
/// selected, or focuses it when it is already open. `onboarding_done` stays true meanwhile.
fn redo_onboarding(windows: &Windows) {
    if let Some(window) = windows.settings.upgrade() {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    }
    if ONBOARDING.with_borrow(Option::is_none) {
        start_onboarding(windows, true);
    } else if let Some(window) = windows.onboarding.upgrade() {
        window
            .window()
            .with_winit_window(|window| window.focus_window());
    }
}

fn refresh_onboarding(windows: &Windows) {
    let (Some(window), Some(onboarding)) = (
        windows.onboarding.upgrade(),
        ONBOARDING.with_borrow(|onboarding| onboarding.clone()),
    ) else {
        return;
    };
    let (shortcut, history, windows_key) = with_app(|app| {
        (
            app.shortcut(),
            app.history_shortcut(),
            app.settings.general.windows_key,
        )
    });
    window.set_step(onboarding.step as i32);
    window.set_selected(onboarding.selected());
    window.set_upgrade(onboarding.upgrade);
    window.set_progress(onboarding.progress());
    window.set_hotkey_text(shortcut.display().into());
    window.set_hotkey_keys(keys_model(&shortcut));
    window.set_history_hotkey_text(history.display().into());
    window.set_done_win(windows_key);
}

/// Answers the current question; the theme question previews its answer at once.
fn onboarding_select(windows: &Windows, option: i32) {
    let Ok(option) = usize::try_from(option) else {
        return;
    };
    let saved = with_app(|app| app.settings.general.theme);
    let preview =
        with_onboarding(|onboarding| onboarding.select(option).then(|| onboarding.theme(saved)))
            .flatten();
    if let Some(theme) = preview {
        with_app(|app| app.theme_preview = Some(theme));
        apply_theme(windows);
    }
    refresh_onboarding(windows);
}

/// Moves forward; leaving the theme question with the default shown previews it.
fn onboarding_next(windows: &Windows, client: &SearchClient) {
    let saved = with_app(|app| app.settings.general.theme);
    let moved = with_onboarding(|onboarding| {
        let before = onboarding.theme(saved);
        let outcome = onboarding.next();
        let after = onboarding.theme(saved);
        (outcome, (after != before).then_some(after))
    });
    if let Some((_, Some(theme))) = moved {
        with_app(|app| app.theme_preview = Some(theme));
        apply_theme(windows);
    }
    match moved.map(|(outcome, _)| outcome) {
        Some(Outcome::Concluded) => conclude_onboarding(windows, client),
        Some(Outcome::OpenSearch) => {
            ONBOARDING.with_borrow_mut(|onboarding| *onboarding = None);
            if let Some(window) = windows.onboarding.upgrade() {
                let _ = window.hide();
            }
            show_popup(windows, client);
            return;
        }
        Some(Outcome::Moved) | None => {}
    }
    refresh_onboarding(windows);
}

/// Writes the answers over the current settings, marks the onboarding done and applies it
/// all through the same path the settings window uses.
fn conclude_onboarding(windows: &Windows, client: &SearchClient) {
    let Some(onboarding) = ONBOARDING.with_borrow(|onboarding| onboarding.clone()) else {
        return;
    };
    let old = with_app(|app| {
        app.theme_preview = None;
        app.settings.clone()
    });
    let mut new = old.clone();
    onboarding.apply(&mut new);
    apply_settings(windows, client, &old, new);
    apply_theme(windows);
    if let Some(window) = windows.settings.upgrade()
        && window.window().is_visible()
    {
        with_app(|app| fill_settings(&window, app));
    }
}

fn wire_onboarding(window: &OnboardingWindow, windows: &Windows, client: &SearchClient) {
    window.on_drag({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                window.window().with_winit_window(|window| {
                    let _ = window.drag_window();
                });
            }
        }
    });
    window.on_select({
        let windows = windows.clone();
        move |option| onboarding_select(&windows, option)
    });
    window.on_next({
        let windows = windows.clone();
        let client = client.clone();
        move || onboarding_next(&windows, &client)
    });
    window.on_back({
        let windows = windows.clone();
        move || {
            with_onboarding(Onboarding::back);
            refresh_onboarding(&windows);
        }
    });
    window.on_skip({
        let windows = windows.clone();
        let client = client.clone();
        move || {
            if with_onboarding(Onboarding::skip) == Some(true) {
                conclude_onboarding(&windows, &client);
            }
            refresh_onboarding(&windows);
        }
    });
    window.on_close_window({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                window
                    .window()
                    .dispatch_event(slint::platform::WindowEvent::CloseRequested);
            }
        }
    });
    window.window().on_close_requested({
        let windows = windows.clone();
        let client = client.clone();
        move || {
            if with_onboarding(Onboarding::skip) == Some(true) {
                conclude_onboarding(&windows, &client);
            }
            ONBOARDING.with_borrow_mut(|onboarding| *onboarding = None);
            CloseRequestResponse::HideWindow
        }
    });
}

fn toggle_popup(windows: &Windows, client: &SearchClient) {
    let Some(popup) = windows.popup.upgrade() else {
        return;
    };
    if popup.window().is_visible() {
        close_popup(&popup);
    } else {
        show_popup(windows, client);
    }
}

fn tray_toggle(windows: &Windows, client: &SearchClient) {
    let Some(popup) = windows.popup.upgrade() else {
        return;
    };
    if popup.window().is_visible() {
        close_popup(&popup);
    } else if !with_app(|app| app::tray_click_closes(app.closed_at, Instant::now())) {
        show_popup(windows, client);
    }
}

fn is_foreground(popup: &Popup) -> bool {
    popup_hwnd(popup) == Some(foreground_window())
}

fn window_hwnd(window: &slint::Window) -> Option<isize> {
    window
        .with_winit_window(
            |window| match window.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
                _ => None,
            },
        )
        .flatten()
}

fn popup_hwnd(popup: &Popup) -> Option<isize> {
    window_hwnd(popup.window())
}

/// Clips a popup window to its "tex -" tab and body.
fn shape_window(window: &slint::Window, tab_width: f32, tab_height: f32) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    let scale = window.scale_factor();
    let size = window.size();
    let radius = theme::design(theme::DEFAULT).shape.radius_tab;
    shape_tabbed_window(
        hwnd,
        size.width as i32,
        size.height as i32,
        placement::physical(tab_width, scale),
        placement::physical(tab_height, scale),
        placement::physical(radius, scale),
    );
}

/// The only way the popup closes, so the time it closed is always known.
fn hide_popup(popup: &Popup) {
    if popup.window().is_visible() {
        with_app(|app| app.closed_at = Some(Instant::now()));
    }
    lock_foreground(false);
    let _ = popup.hide();
}

/// Closes the popup from the keyboard or a toggle, giving the foreground back to the window
/// that had it instead of letting Windows pick the next one. Hiding because the focus went
/// elsewhere uses [`hide_popup`], so the window the user chose keeps it.
fn close_popup(popup: &Popup) {
    let previous = with_app(|app| std::mem::take(&mut app.previous_foreground));
    if popup.window().is_visible() && is_foreground(popup) {
        lock_foreground(false);
        restore_foreground(previous);
    }
    hide_popup(popup);
}

/// Restores the last search if it is recent enough (otherwise clears it), asks the service
/// for its status, moves the popup to the configured monitor and gives it keyboard focus.
/// The popup only closes when it loses the focus, so one that cannot get it is hidden again.
fn show_popup(windows: &Windows, client: &SearchClient) {
    let Some(popup) = windows.popup.upgrade() else {
        return;
    };
    apply_theme(windows);
    let (keep, monitor) = with_app(|app| {
        let keep = app.closed_at.is_some_and(|closed_at| {
            app.settings
                .search
                .keep_last_search
                .keeps(closed_at, Instant::now())
        });
        (keep, app.settings.general.popup_monitor)
    });
    let keep = keep && !popup.get_query().trim().is_empty();
    if !keep {
        popup.invoke_reset();
        popup.set_rows(ModelRc::default());
        popup.set_selected(-1);
    }
    popup.set_status_indexing("".into());
    popup.set_status_failed("".into());
    popup.set_status_ready_letters("".into());
    popup.set_service_unavailable(false);
    client.send(Command::Status);
    check_updates_if_stale(windows);
    let area = match monitor {
        PopupMonitor::Cursor => work_area_under_cursor(),
        PopupMonitor::Primary => primary_work_area(),
    };
    if let Some(area) = area {
        let (x, y) = placement::popup_origin(&area, placement::POPUP_WIDTH);
        popup.window().set_position(PhysicalPosition::new(x, y));
    }
    let previous = foreground_window();
    with_app(|app| app.previous_foreground = previous);
    if popup.show().is_err() {
        return;
    }
    popup
        .window()
        .with_winit_window(|window| window.set_skip_taskbar(true));
    shape_window(
        popup.window(),
        popup.get_tab_width(),
        popup.get_tab_height(),
    );
    let Some(window) = popup_hwnd(&popup).filter(|window| take_foreground(*window)) else {
        hide_popup(&popup);
        return;
    };
    {
        let windows = windows.clone();
        catch_windows_key(window, move || {
            if !with_app(|app| app.settings.winkey_active()) {
                return false;
            }
            let windows = windows.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(popup) = windows.popup.upgrade() {
                    close_popup(&popup);
                }
            });
            true
        });
    }
    lock_foreground(true);
    if keep {
        popup.invoke_restore();
        search(&popup, client);
    }
}

fn search(popup: &Popup, client: &SearchClient) {
    let query = popup.get_query();
    popup.set_expanded(!query.trim().is_empty());
    if query.trim().is_empty() {
        popup.set_rows(ModelRc::default());
        popup.set_selected(-1);
        return;
    }
    let filter = match popup.get_filter() {
        1 => Filter::Apps,
        2 => Filter::Files,
        3 => Filter::Folders,
        _ => Filter::All,
    };
    popup.set_searching(true);
    client.search(
        query.to_string(),
        filter,
        with_app(|app| app::search_options(&app.settings.search)),
    );
}

fn move_selection(popup: &Popup, step: i32) {
    let is_item: Vec<bool> = popup.get_rows().iter().map(|row| !row.header).collect();
    let current = usize::try_from(popup.get_selected()).ok();
    if let Some(next) = step_selection(&is_item, current, step as isize) {
        popup.set_selected(next as i32);
    }
}

fn show_event(windows: &Windows, event: Event, icons: &IconLoader, latest: &Latest) {
    let popup = windows.popup.upgrade();
    let settings = windows.settings.upgrade();
    match event {
        Event::Results(results) => {
            let Some(popup) = popup else {
                return;
            };
            if popup.get_query().trim().is_empty() || !latest.is(results.id) {
                return;
            }
            let rows = build_rows(&results.hits);
            let is_item: Vec<bool> = rows.iter().map(Row::is_item).collect();
            let selected = step_selection(&is_item, None, 1);
            let counts = header_counts(&rows);
            let viewer_exts = reader_client::viewer_extensions();
            let model: Vec<ResultRow> = rows
                .iter()
                .zip(counts)
                .map(|(row, count)| to_result_row(row, count, icons, &viewer_exts))
                .collect();
            popup.set_rows(ModelRc::new(VecModel::from(model)));
            popup.set_selected(selected.map_or(-1, |index| index as i32));
            popup.set_service_unavailable(false);
            popup.set_searching(false);
        }
        Event::Status(volumes) => {
            let separator = app::digit_separator(with_app(|app| app.settings.general.language));
            if let Some(popup) = popup {
                let (indexing, failed) = status_parts(&volumes);
                popup.set_status_indexing(indexing.into());
                popup.set_status_failed(failed.into());
                let (letters, total) = ready_summary(&volumes);
                popup.set_status_ready_letters(letters.into());
                popup.set_status_ready_total(group_digits(total, separator).into());
                popup.set_service_unavailable(false);
            }
            if let Some(settings) = settings {
                let drives: Vec<DriveRow> = volumes
                    .iter()
                    .map(|volume| drive_row(volume, separator))
                    .collect();
                settings.set_drives(ModelRc::new(VecModel::from(drives)));
                settings.set_drives_unavailable(false);
            }
        }
        Event::Unavailable => {
            if let Some(popup) = popup {
                popup.set_rows(ModelRc::default());
                popup.set_selected(-1);
                popup.set_service_unavailable(true);
                popup.set_searching(false);
            }
            if let Some(settings) = settings {
                settings.set_drives(ModelRc::default());
                settings.set_drives_unavailable(true);
            }
        }
    }
}

fn drive_row(volume: &VolumeStatus, separator: char) -> DriveRow {
    let mut row = DriveRow {
        letter: volume.letter.to_string().into(),
        ..Default::default()
    };
    match &volume.state {
        VolumeState::Indexing { percent } => {
            row.state = 0;
            row.percent = i32::from(*percent);
        }
        VolumeState::Ready { entries } => {
            row.state = 1;
            row.entries = group_digits(u64::from(*entries), separator).into();
        }
        VolumeState::Failed { reason } => {
            row.state = 2;
            row.reason = reason.as_str().into();
        }
    }
    row
}

fn to_result_row(row: &Row, count: usize, icons: &IconLoader, viewer_exts: &[String]) -> ResultRow {
    match row {
        Row::Header(category) => ResultRow {
            header: true,
            category: category.index(),
            count: count as i32,
            ..Default::default()
        },
        Row::Item {
            name,
            folder,
            path,
            is_dir,
            is_app,
        } => {
            let mut result = ResultRow {
                header: false,
                title: name.as_str().into(),
                folder: folder.as_str().into(),
                short_folder: short_path(folder).into(),
                path: path.as_str().into(),
                is_dir: *is_dir,
                is_app: *is_app,
                readable: readable(name, *is_dir, *is_app, viewer_exts),
                file_icon: file_icon(name).index(),
                ..Default::default()
            };
            if *is_app {
                ICONS.with_borrow_mut(|cache| match cache.get(path) {
                    Some(IconState::Loaded(icon)) => {
                        result.icon = icon.clone();
                        result.has_icon = true;
                    }
                    Some(_) => {}
                    None => {
                        if cache.mark_requested(path) {
                            icons.request(path.clone());
                        }
                    }
                });
            }
            result
        }
    }
}

/// Opens the row, or shows it in its folder. `alternate` (Ctrl+Enter) does the action the
/// Enter key setting does not.
fn activate(popup: &Popup, index: i32, alternate: bool) {
    let Some(row) = usize::try_from(index)
        .ok()
        .and_then(|index| popup.get_rows().row_data(index))
    else {
        return;
    };
    if row.header {
        return;
    }
    let reveal_by_default = with_app(|app| app.settings.search.enter_action == EnterAction::Reveal);
    let opened = if alternate != reveal_by_default {
        reveal_in_folder(&row.path)
    } else {
        open_path(&row.path)
    };
    if opened.is_ok() {
        hide_popup(popup);
    }
}

/// Stores a freshly loaded icon and puts it on every visible row for that path.
fn icon_loaded(popup: &Popup, path: String, pixels: Option<IconPixels>) {
    let image = pixels.map(|pixels| {
        Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        ))
    });
    ICONS.with_borrow_mut(|cache| cache.store(path.clone(), image.clone()));
    let Some(image) = image else {
        return;
    };
    let rows = popup.get_rows();
    for index in 0..rows.row_count() {
        if let Some(mut row) = rows.row_data(index)
            && row.is_app
            && row.path == path.as_str()
        {
            row.icon = image.clone();
            row.has_icon = true;
            rows.set_row_data(index, row);
        }
    }
}

fn wire_history(window: &HistoryWindow, windows: &Windows, client: &SearchClient) {
    window.on_dismissed({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade() {
                hide_history(&window);
            }
        }
    });
    window.window().on_winit_window_event({
        let window = window.as_weak();
        let windows = windows.clone();
        move |_, event| {
            match event {
                WindowEvent::Focused(false) => {
                    let _ = window.upgrade_in_event_loop(|window| {
                        if window.window().is_visible()
                            && !window.get_renaming()
                            && !window.get_editing()
                        {
                            hide_history(&window);
                        }
                    });
                }
                WindowEvent::ThemeChanged(_) => theme_changed(&windows),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    let _ = window.upgrade_in_event_loop(|window| {
                        shape_window(
                            window.window(),
                            window.get_tab_width(),
                            window.get_tab_height(),
                        );
                    });
                }
                _ => {}
            }
            EventResult::Propagate
        }
    });
    window.on_toggle_pause({
        let windows = windows.clone();
        move || toggle_pause(&windows)
    });
    window.on_query_edited({
        let window = window.as_weak();
        move |_| {
            if let Some(window) = window.upgrade() {
                load_history(&window);
            }
        }
    });
    window.on_filter_changed({
        let window = window.as_weak();
        move |_| {
            if let Some(window) = window.upgrade() {
                load_history(&window);
            }
        }
    });
    window.on_step({
        let window = window.as_weak();
        move |step| {
            if let Some(window) = window.upgrade() {
                with_history(|history| history.list.step(step as isize));
                sync_selection(&window);
            }
        }
    });
    window.on_select({
        let window = window.as_weak();
        move |index| {
            if let (Some(window), Ok(index)) = (window.upgrade(), usize::try_from(index)) {
                with_history(|history| history.list.select_index(index));
                sync_selection(&window);
            }
        }
    });
    window.on_pick({
        let window = window.as_weak();
        move |index, alternate| {
            if let Some(window) = window.upgrade() {
                pick(&window, index, alternate);
            }
        }
    });
    window.on_toggle_favorite({
        let window = window.as_weak();
        move |index| {
            let Some(row) = window.upgrade().and_then(|window| row_at(&window, index)) else {
                return;
            };
            if let Ok(id) = row.id.parse() {
                send_history(ClipboardCommand::SetFavorite(id, !row.favorite));
            }
        }
    });
    window.on_rename({
        let window = window.as_weak();
        move |index, name| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.set_renaming(false);
            if let Some(id) = row_id(&window, index) {
                let name = (!name.trim().is_empty()).then(|| name.to_string());
                send_history(ClipboardCommand::Rename(id, name));
            }
        }
    });
    window.on_edit({
        let window = window.as_weak();
        move |index, text| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.set_action_error(0);
            if let Some(id) = row_id(&window, index) {
                with_history(|history| history.edit = Some((id, text.to_string())));
                send_history(ClipboardCommand::EditText(id, text.to_string()));
            }
        }
    });
    window.on_delete({
        let window = window.as_weak();
        move |index| {
            if let Some(id) = window.upgrade().and_then(|window| row_id(&window, index)) {
                send_history(ClipboardCommand::Delete(id));
            }
        }
    });
    window.on_load_more({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade() {
                load_more(&window);
            }
        }
    });
    window.on_open_settings({
        let windows = windows.clone();
        let client = client.clone();
        move || {
            if let Some(window) = windows.history.upgrade() {
                hide_history(&window);
            }
            show_settings(&windows, &client, Some(CLIPBOARD_TAB));
        }
    });
}

fn send_history(command: ClipboardCommand) {
    with_history(|history| history.client.send(command));
}

fn row_at(window: &HistoryWindow, index: i32) -> Option<HistoryRow> {
    window.get_rows().row_data(usize::try_from(index).ok()?)
}

fn row_id(window: &HistoryWindow, index: i32) -> Option<i64> {
    row_at(window, index)?.id.parse().ok()
}

fn toggle_history(windows: &Windows) {
    let Some(window) = windows.history.upgrade() else {
        return;
    };
    if window.window().is_visible() {
        hide_history(&window);
    } else {
        show_history(windows);
    }
}

/// Remembers the window to paste into, clears the search, loads the newest rows and opens
/// the history on the configured monitor with keyboard focus.
fn show_history(windows: &Windows) {
    let Some(window) = windows.history.upgrade() else {
        return;
    };
    let target = foreground_window();
    apply_theme(windows);
    let (monitor, paused, paste) = with_app(|app| {
        app.paste_target = target;
        (
            app.settings.general.popup_monitor,
            app::capture_paused(&app.settings),
            app.settings.clipboard.on_pick == OnPick::CopyAndPaste,
        )
    });
    window.invoke_reset();
    window.set_paused(paused);
    window.set_paste_by_default(paste);
    window.set_unavailable(with_history(|history| history.unavailable.clone()).into());
    window.set_current_year(i32::from(local_time(now_ms()).year));
    sync_selection(&window);
    load_history(&window);
    let area = match monitor {
        PopupMonitor::Cursor => work_area_under_cursor(),
        PopupMonitor::Primary => primary_work_area(),
    };
    if let Some(area) = area {
        let (x, y) = placement::popup_origin(&area, placement::HISTORY_WIDTH);
        window.window().set_position(PhysicalPosition::new(x, y));
    }
    if window.show().is_err() {
        return;
    }
    window.window().with_winit_window(|window| {
        window.set_skip_taskbar(true);
        window.focus_window();
    });
    shape_window(
        window.window(),
        window.get_tab_width(),
        window.get_tab_height(),
    );
}

/// Hides the history and drops its rows and content, so nothing stays in memory while closed.
fn hide_history(window: &HistoryWindow) {
    let _ = window.hide();
    window.set_renaming(false);
    window.set_editing(false);
    window.set_confirm_delete(false);
    clear_content(window);
    with_history(|history| {
        history.client.cancel_search();
        history.list = HistoryList::default();
        history.rows.set_vec(Vec::new());
        history.search = None;
        history.more = false;
        history.loading_more = false;
        history.shown = None;
        history.content = None;
        history.pick = None;
        history.edit = None;
    });
}

fn history_filter(window: &HistoryWindow) -> HistoryFilter {
    match window.get_filter() {
        1 => HistoryFilter::Favorites,
        2 => HistoryFilter::Text,
        3 => HistoryFilter::Images,
        _ => HistoryFilter::All,
    }
}

/// Asks for the rows of the current query and filter: the newest page, or a search.
fn load_history(window: &HistoryWindow) {
    let query = window.get_query().to_string();
    let filter = history_filter(window);
    window.set_searching(true);
    with_history(|history| {
        history.more = false;
        history.loading_more = false;
        if query.trim().is_empty() {
            history.client.cancel_search();
            history.search = None;
            history.client.send(ClipboardCommand::Page {
                filter,
                before: None,
            });
        } else {
            history.search = Some(history.client.search(query, filter));
        }
    });
}

/// Asks for the next page of the list or of the search when it may continue and none is on
/// its way.
fn load_more(window: &HistoryWindow) {
    let filter = history_filter(window);
    let query = window.get_query().to_string();
    with_history(|history| {
        if !history.more || history.loading_more {
            return;
        }
        let Some(before) = history.list.last() else {
            return;
        };
        history.loading_more = true;
        match history.search {
            Some(generation) => history
                .client
                .search_more(query, filter, before, generation),
            None => history.client.send(ClipboardCommand::Page {
                filter,
                before: Some(before),
            }),
        }
    });
}

fn clipboard_event(windows: &Windows, event: ClipboardEvent) {
    if let Some(settings) = windows.settings.upgrade() {
        match &event {
            ClipboardEvent::Usage(bytes) => {
                let decimal = app::decimal_separator(with_app(|app| app.settings.general.language));
                let size = size_parts(*bytes, decimal);
                settings.set_usage_value(size.value.into());
                settings.set_usage_unit(size.unit);
            }
            ClipboardEvent::Unavailable(reason) => {
                settings.set_history_unavailable(reason.as_str().into());
            }
            _ => {}
        }
    }
    let Some(window) = windows.history.upgrade() else {
        return;
    };
    let visible = window.window().is_visible();
    match event {
        ClipboardEvent::Unavailable(reason) => {
            window.set_unavailable(reason.as_str().into());
            window.set_searching(false);
            with_history(|history| history.unavailable = reason);
        }
        ClipboardEvent::Page { rows, append } => {
            let (current, loading_more) =
                with_history(|history| (history.search.is_none(), history.loading_more));
            if !visible || !current {
                return;
            }
            if append {
                if loading_more {
                    append_rows(&window, &rows, PAGE_SIZE);
                }
            } else {
                let more = rows.len() == PAGE_SIZE;
                replace_rows(&window, &rows, more);
            }
        }
        ClipboardEvent::SearchResults {
            generation,
            rows,
            append,
        } => {
            let (current, loading_more) =
                with_history(|history| (history.search == Some(generation), history.loading_more));
            if !visible || !current {
                return;
            }
            if append {
                if loading_more {
                    append_rows(&window, &rows, SEARCH_LIMIT);
                }
            } else {
                replace_rows(&window, &rows, rows.len() == SEARCH_LIMIT);
            }
        }
        ClipboardEvent::Added(summary, found) => {
            if visible {
                row_added(&window, &summary, found);
            }
        }
        ClipboardEvent::Changed(summary) => {
            if visible {
                row_changed(&window, &summary);
            }
        }
        ClipboardEvent::Removed(ids) => {
            if visible {
                remove_rows(&window, &ids);
            }
        }
        ClipboardEvent::Content(id, content) => content_arrived(&window, visible, id, content),
        ClipboardEvent::EditRefused(id, error) => {
            if visible {
                edit_refused(&window, id, error);
            }
        }
        ClipboardEvent::Usage(_) => {}
    }
}

fn history_rows(rows: &[Summary]) -> Vec<HistoryRow> {
    let now = local_time(now_ms());
    let decimal = app::decimal_separator(with_app(|app| app.settings.general.language));
    rows.iter()
        .map(|row| history_row(row, now, decimal))
        .collect()
}

fn history_row(summary: &Summary, now: LocalTime, decimal: char) -> HistoryRow {
    let when = when_parts(local_time(summary.copied_at), now);
    let size = size_parts(summary.size, decimal);
    let pixels =
        |value: Option<u32>| value.map_or(0, |value| i32::try_from(value).unwrap_or(i32::MAX));
    let mut row = HistoryRow {
        id: summary.id.to_string().into(),
        kind: if summary.kind == Kind::Image { 1 } else { 0 },
        named: summary.name.is_some(),
        favorite: summary.favorite,
        day: when.day,
        time: when.time.into(),
        date_day: when.date[0],
        date_month: when.date[1],
        date_year: when.date[2],
        size_value: size.value.into(),
        size_unit: size.unit,
        width: pixels(summary.width),
        height: pixels(summary.height),
        ..Default::default()
    };
    match &summary.preview {
        Preview::Text(text) => row.title = summary.name.as_deref().unwrap_or(text).into(),
        Preview::Image(png) => {
            row.title = summary.name.as_deref().unwrap_or_default().into();
            if let Some(thumbnail) = decode_png(png) {
                row.thumbnail = rgba_image(&thumbnail);
            }
        }
        Preview::Unreadable => row.unreadable = true,
    }
    row
}

fn rgba_image(image: &Rgba) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &image.pixels,
        image.width,
        image.height,
    ))
}

/// Shows a new list (a page or search results) and selects its first row.
fn replace_rows(window: &HistoryWindow, rows: &[Summary], more: bool) {
    let model = history_rows(rows);
    with_history(|history| {
        history
            .list
            .replace(rows.iter().map(|row| row.id).collect());
        history.rows.set_vec(model);
        history.more = more;
        history.loading_more = false;
    });
    window.set_searching(false);
    window.invoke_scroll_to_top();
    sync_selection(window);
}

fn append_rows(window: &HistoryWindow, rows: &[Summary], page: usize) {
    let model = history_rows(rows);
    with_history(|history| {
        history.list.append(rows.iter().map(|row| row.id).collect());
        for row in model {
            history.rows.push(row);
        }
        history.more = rows.len() == page;
        history.loading_more = false;
    });
    sync_selection(window);
}

/// A new copy goes on top when the filter takes it and, during a search, the search finds it.
fn row_added(window: &HistoryWindow, summary: &Summary, found: bool) {
    let accepted = match history_filter(window) {
        HistoryFilter::All => true,
        HistoryFilter::Favorites => summary.favorite,
        HistoryFilter::Text => summary.kind == Kind::Text,
        HistoryFilter::Images => summary.kind == Kind::Image,
    };
    if !accepted || (!found && with_history(|history| history.search.is_some())) {
        return;
    }
    let row = history_rows(std::slice::from_ref(summary)).remove(0);
    with_history(|history| {
        history.list.insert_top(summary.id);
        history.rows.insert(0, row);
    });
    sync_selection(window);
}

/// Updates a row in place; an unfavorited row leaves the favorites filter. A saved edit
/// closes the editor and becomes the shown content.
fn row_changed(window: &HistoryWindow, summary: &Summary) {
    let edited = with_history(|history| {
        let saved = history
            .edit
            .take_if(|(id, _)| *id == summary.id)
            .map(|(id, text)| history.content = Some((id, Some(Content::Text(text)))));
        saved.is_some()
    });
    if edited {
        window.set_editing(false);
        window.set_action_error(0);
        show_content(window);
    }
    if history_filter(window) == HistoryFilter::Favorites && !summary.favorite {
        remove_rows(window, &[summary.id]);
        return;
    }
    let row = history_rows(std::slice::from_ref(summary)).remove(0);
    with_history(|history| {
        if let Some(index) = history.list.index_of(summary.id) {
            history.rows.set_row_data(index, row);
        }
    });
}

fn remove_rows(window: &HistoryWindow, ids: &[i64]) {
    let gone: HashSet<i64> = ids.iter().copied().collect();
    with_history(|history| {
        for index in (0..history.rows.row_count()).rev() {
            let removed = history
                .rows
                .row_data(index)
                .and_then(|row| row.id.parse().ok())
                .is_some_and(|id| gone.contains(&id));
            if removed {
                history.rows.remove(index);
            }
        }
        history.list.remove(ids);
        if history
            .content
            .as_ref()
            .is_some_and(|(id, _)| gone.contains(id))
        {
            history.content = None;
        }
    });
    sync_selection(window);
}

/// Puts the list's selection on screen; a new selection closes the editors and shows its content.
fn sync_selection(window: &HistoryWindow) {
    let (index, changed) = with_history(|history| {
        let selected = history.list.selected();
        let changed = selected != history.shown;
        history.shown = selected;
        (history.list.selected_index(), changed)
    });
    window.set_selected(index.map_or(-1, |index| index as i32));
    let headers = index.map_or(0, |index| {
        let dates: Vec<[i32; 3]> = window
            .get_rows()
            .iter()
            .map(|row| [row.date_day, row.date_month, row.date_year])
            .collect();
        day_starts_through(&dates, index)
    });
    window.set_selected_headers(headers as i32);
    if changed {
        window.set_expanded(true);
        window.set_renaming(false);
        window.set_editing(false);
        window.set_confirm_delete(false);
        window.set_action_error(0);
        show_content(window);
    }
}

/// Shows the selected row's content when it is known, otherwise asks the worker for it.
fn show_content(window: &HistoryWindow) {
    with_history(|history| {
        let Some(id) = history.shown else {
            clear_content(window);
            return;
        };
        match &history.content {
            Some((cached, content)) if *cached == id => set_content(window, content.as_ref()),
            _ => {
                clear_content(window);
                request_content(history, id);
            }
        }
    });
}

/// Only one content request is on its way at a time; the newest selection is asked for
/// when it arrives.
fn request_content(history: &mut HistoryState, id: i64) {
    if history.requested.is_none() {
        history.requested = Some(id);
        history.client.send(ClipboardCommand::Content(id));
    }
}

/// Content that does not decrypt makes its row unreadable, so it can only be deleted.
fn content_arrived(window: &HistoryWindow, visible: bool, id: i64, content: Option<Content>) {
    let paste = with_history(|history| {
        history.requested = None;
        if !visible {
            return None;
        }
        if content.is_none()
            && let Some(index) = history.list.index_of(id)
            && let Some(mut row) = history.rows.row_data(index)
        {
            row.unreadable = true;
            history.rows.set_row_data(index, row);
        }
        history.content = Some((id, content));
        match history.shown {
            Some(shown) if shown == id => {
                set_content(
                    window,
                    history
                        .content
                        .as_ref()
                        .and_then(|(_, content)| content.as_ref()),
                );
            }
            Some(shown) => request_content(history, shown),
            None => {}
        }
        history
            .pick
            .take_if(|(pick, _)| *pick == id)
            .map(|(_, paste)| paste)
    });
    if let Some(paste) = paste {
        finish_pick(window, paste);
    }
}

fn clear_content(window: &HistoryWindow) {
    window.set_content_kind(-1);
    window.set_content_text(SharedString::default());
    window.set_content_preview(SharedString::default());
    window.set_content_image(Image::default());
}

fn set_content(window: &HistoryWindow, content: Option<&Content>) {
    clear_content(window);
    match content {
        Some(Content::Text(text)) => {
            window.set_content_text(text.as_str().into());
            window.set_content_preview(preview(text).into());
            window.set_content_kind(0);
        }
        Some(Content::Image { png, .. }) => match decode_png(png) {
            Some(image) => {
                window.set_content_image(rgba_image(&image));
                window.set_content_kind(1);
            }
            None => window.set_content_kind(2),
        },
        None => window.set_content_kind(2),
    }
}

/// The start of a long text, so the panel lays out quickly.
fn preview(text: &str) -> String {
    match text.char_indices().nth(PREVIEW_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// Copies the row (and pastes when the setting and `alternate` say so), once its content
/// is known.
fn pick(window: &HistoryWindow, index: i32, alternate: bool) {
    if let Ok(index) = usize::try_from(index) {
        with_history(|history| history.list.select_index(index));
    }
    sync_selection(window);
    let paste = with_app(|app| app.settings.clipboard.on_pick == OnPick::CopyAndPaste) != alternate;
    let ready = with_history(|history| {
        let Some(id) = history.shown else {
            return false;
        };
        if matches!(&history.content, Some((cached, _)) if *cached == id) {
            return true;
        }
        history.pick = Some((id, paste));
        request_content(history, id);
        false
    });
    if ready {
        finish_pick(window, paste);
    }
}

/// Writes the shown content to the clipboard. On success the history closes and, for
/// paste, the window that had the focus gets it back with Ctrl+V.
fn finish_pick(window: &HistoryWindow, paste: bool) {
    let written = with_history(|history| match &history.content {
        Some((id, Some(content))) if history.shown == Some(*id) => Some(write_content(content)),
        _ => None,
    });
    match written {
        Some(Ok(())) => {
            hide_history(window);
            if paste {
                paste_into(with_app(|app| app.paste_target));
            }
        }
        Some(Err(_)) => window.set_action_error(2),
        None => {}
    }
}

fn write_content(content: &Content) -> io::Result<()> {
    match content {
        Content::Text(text) => write_text(text),
        Content::Image { png, .. } => {
            let image =
                decode_png(png).ok_or_else(|| io::Error::other("the image does not decode"))?;
            write_image(&rgba_to_dib(&image), png)
        }
    }
}

fn edit_refused(window: &HistoryWindow, id: i64, error: EditError) {
    let pending = with_history(|history| history.edit.take_if(|(edit, _)| *edit == id).is_some());
    if !pending {
        return;
    }
    if error == EditError::TooLarge {
        window.set_action_error(1);
    } else {
        window.set_editing(false);
    }
}
