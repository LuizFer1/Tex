#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clipboard_client;
mod history;
mod icon;
mod icons;
mod placement;
mod results;
mod search_client;
mod settings;
mod shortcut;

use std::cell::RefCell;
use std::collections::HashSet;
use std::error::Error;
use std::io;
use std::rc::Rc;
use std::time::Instant;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
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
    ClipboardWatcher, IconPixels, LocalTime, SingleInstance, foreground_window, local_time,
    open_path, paste_into, pick_folder, primary_work_area, reveal_in_folder,
    work_area_under_cursor, write_image, write_text,
};

use crate::app::{AppState, HotkeyError, with_app};
use crate::clipboard_client::{
    ClipboardClient, Command as ClipboardCommand, Event as ClipboardEvent, PAGE_SIZE,
    history_folder, now_ms,
};
use crate::history::{HistoryList, size_parts, when_parts};
use crate::icons::{IconCache, IconLoader, IconState};
use crate::results::{Row, build_rows, group_digits, status_parts, step_selection};
use crate::search_client::{Command, Event, SearchClient};
use crate::settings::{
    EnterAction, KeepSearch, Language, OnPick, PopupMonitor, Settings, Theme, settings_path,
};
use crate::shortcut::Shortcut;

slint::include_modules!();

const ICON_CACHE_SIZE: usize = 256;
const CLIPBOARD_TAB: i32 = 2;
const PREVIEW_CHARS: usize = 100_000;

thread_local! {
    static ICONS: RefCell<IconCache<Image>> = RefCell::new(IconCache::new(ICON_CACHE_SIZE));
    static HISTORY: RefCell<Option<HistoryState>> = const { RefCell::new(None) };
}

/// Weak handles to every window, cheap to clone into callbacks and across threads.
#[derive(Clone)]
struct Windows {
    popup: Weak<Popup>,
    about: Weak<AboutWindow>,
    settings: Weak<SettingsWindow>,
    history: Weak<HistoryWindow>,
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

/// Runs `f` on the history state. Never call into Slint callbacks from inside.
fn with_history<R>(f: impl FnOnce(&mut HistoryState) -> R) -> R {
    HISTORY.with_borrow_mut(|history| f(history.as_mut().expect("history state initialized")))
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = settings_path();
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
    about.set_version(env!("CARGO_PKG_VERSION").into());
    tray.set_icon_image(icon::tray_icon());
    let windows = Windows {
        popup: popup.as_weak(),
        about: about.as_weak(),
        settings: settings_window.as_weak(),
        history: history.as_weak(),
    };

    app::init(AppState::new(settings, path, GlobalHotKeyManager::new()?));
    let language = with_app(|app| app.settings.general.language);
    app::apply_language(language);
    apply_theme(&windows);
    with_app(|app| apply_categories(&popup, &app.settings));

    let clipboard = ClipboardClient::start(
        history_folder(),
        with_app(|app| app::client_settings(&app.settings.clipboard)),
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
    let _watcher = match ClipboardWatcher::start({
        let clipboard = clipboard.clone();
        move |clip| clipboard.send(ClipboardCommand::Captured(clip))
    }) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            with_app(|app| app.capture_error = error.to_string());
            None
        }
    };

    let icons = IconLoader::start({
        let popup = popup.as_weak();
        move |path, pixels| {
            let _ = popup.upgrade_in_event_loop(move |popup| icon_loaded(&popup, path, pixels));
        }
    });

    let client = SearchClient::start(std::env::var("USERPROFILE").unwrap_or_default(), {
        let windows = windows.clone();
        let icons = icons.clone();
        move |event| {
            let windows = windows.clone();
            let icons = icons.clone();
            let _ = slint::invoke_from_event_loop(move || show_event(&windows, event, &icons));
        }
    });

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
    if with_app(|app| app.settings.general.windows_key) {
        let on_press = winkey_handler(&windows, &client);
        if with_app(|app| app.set_winkey(true, on_press)).is_err() {
            with_app(|app| app.settings.general.windows_key = false);
        }
    }

    wire_popup(&popup, &windows, &client);
    wire_settings(&settings_window, &windows, &client);
    wire_history(&history, &windows, &client);

    tray.on_open({
        let windows = windows.clone();
        let client = client.clone();
        move || show_popup(&windows, &client)
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
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.show()?;

    slint::run_event_loop_until_quit()?;
    Ok(())
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
                hide_popup(&popup);
            }
        }
    });
    popup.window().on_winit_window_event({
        let popup = popup.as_weak();
        move |_, event| {
            if matches!(event, WindowEvent::Focused(false)) {
                let _ = popup.upgrade_in_event_loop(|popup| hide_popup(&popup));
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
    window.on_edited({
        let windows = windows.clone();
        let client = client.clone();
        move || settings_edited(&windows, &client)
    });
    window.on_change_hotkey({
        let window = window.as_weak();
        move || {
            with_app(|app| app.suspend_hotkey());
            if let Some(window) = window.upgrade() {
                window.set_hotkey_error(0);
                window.set_recording(true);
            }
        }
    });
    window.on_cancel_recording({
        let window = window.as_weak();
        move || {
            with_app(|app| app.resume_hotkey());
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
    window.window().on_close_requested({
        let window = window.as_weak();
        move || {
            if let Some(window) = window.upgrade()
                && window.get_recording()
            {
                window.set_recording(false);
                with_app(|app| app.resume_hotkey());
            }
            CloseRequestResponse::HideWindow
        }
    });
}

fn change_hotkey(window: &SettingsWindow, shortcut: &Shortcut) {
    let result = with_app(|app| app.apply_hotkey(shortcut));
    window.set_recording(false);
    window.set_hotkey_error(match result {
        Ok(()) => 0,
        Err(HotkeyError::Register) => 1,
        Err(HotkeyError::Conflict) => 2,
    });
    window.set_hotkey_text(with_app(|app| app.shortcut().display()).into());
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

/// Reads the settings window back, applies whatever changed and saves.
fn settings_edited(windows: &Windows, client: &SearchClient) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    let old = with_app(|app| app.settings.clone());
    let mut new = read_settings(&window, &old);
    if new.general.windows_key != old.general.windows_key {
        let on_press = winkey_handler(windows, client);
        let result = with_app(|app| app.set_winkey(new.general.windows_key, on_press));
        if result.is_err() {
            new.general.windows_key = false;
            window.set_windows_key(false);
        }
        window.set_winkey_error(with_app(|app| app.winkey_error.clone()).into());
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
    save_settings(&window);
}

fn apply_theme(windows: &Windows) {
    let scheme = app::color_scheme(with_app(|app| app.settings.general.theme));
    if let Some(popup) = windows.popup.upgrade() {
        popup.set_scheme(scheme);
    }
    if let Some(about) = windows.about.upgrade() {
        about.set_scheme(scheme);
    }
    if let Some(settings) = windows.settings.upgrade() {
        settings.set_scheme(scheme);
    }
    if let Some(history) = windows.history.upgrade() {
        history.set_scheme(scheme);
    }
}

fn apply_categories(popup: &Popup, settings: &Settings) {
    let search = &settings.search;
    popup.set_show_apps(search.show_apps);
    popup.set_show_files(search.show_files);
    popup.set_show_folders(search.show_folders);
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
    window.set_hotkey_error(if app.hotkey_failed { 1 } else { 0 });
    window.set_recording(false);
    window.set_windows_key(general.windows_key);
    window.set_winkey_error(app.winkey_error.as_str().into());
    window.set_start_with_windows(general.start_with_windows);
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
    if let Some(tab) = tab {
        window.set_current_tab(tab);
    }
    client.send(Command::Status);
    let _ = window.show();
    window
        .window()
        .with_winit_window(|window| window.focus_window());
}

fn toggle_popup(windows: &Windows, client: &SearchClient) {
    let Some(popup) = windows.popup.upgrade() else {
        return;
    };
    if popup.window().is_visible() {
        hide_popup(&popup);
    } else {
        show_popup(windows, client);
    }
}

/// The only way the popup closes, so the time it closed is always known.
fn hide_popup(popup: &Popup) {
    if popup.window().is_visible() {
        with_app(|app| app.closed_at = Some(Instant::now()));
    }
    let _ = popup.hide();
}

/// Restores the last search if it is recent enough (otherwise clears it), asks the service
/// for its status, moves the popup to the configured monitor and gives it keyboard focus.
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
    popup.set_service_unavailable(false);
    client.send(Command::Status);
    let area = match monitor {
        PopupMonitor::Cursor => work_area_under_cursor(),
        PopupMonitor::Primary => primary_work_area(),
    };
    if let Some(area) = area {
        let (x, y) = placement::popup_origin(&area, placement::POPUP_WIDTH);
        popup.window().set_position(PhysicalPosition::new(x, y));
    }
    if popup.show().is_err() {
        return;
    }
    popup.window().with_winit_window(|window| {
        window.set_skip_taskbar(true);
        window.focus_window();
    });
    if keep {
        popup.invoke_restore();
        search(&popup, client);
    }
}

fn search(popup: &Popup, client: &SearchClient) {
    let query = popup.get_query();
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
    client.send(Command::Search {
        query: query.to_string(),
        filter,
        options: with_app(|app| app::search_options(&app.settings.search)),
    });
}

fn move_selection(popup: &Popup, step: i32) {
    let is_item: Vec<bool> = popup.get_rows().iter().map(|row| !row.header).collect();
    let current = usize::try_from(popup.get_selected()).ok();
    if let Some(next) = step_selection(&is_item, current, step as isize) {
        popup.set_selected(next as i32);
    }
}

fn show_event(windows: &Windows, event: Event, icons: &IconLoader) {
    let popup = windows.popup.upgrade();
    let settings = windows.settings.upgrade();
    match event {
        Event::Results(results) => {
            let Some(popup) = popup else {
                return;
            };
            if popup.get_query().trim().is_empty() {
                return;
            }
            let rows = build_rows(&results.hits);
            let is_item: Vec<bool> = rows.iter().map(Row::is_item).collect();
            let selected = step_selection(&is_item, None, 1);
            let model: Vec<ResultRow> = rows.iter().map(|row| to_result_row(row, icons)).collect();
            popup.set_rows(ModelRc::new(VecModel::from(model)));
            popup.set_selected(selected.map_or(-1, |index| index as i32));
            popup.set_service_unavailable(false);
        }
        Event::Status(volumes) => {
            if let Some(popup) = popup {
                let (indexing, failed) = status_parts(&volumes);
                popup.set_status_indexing(indexing.into());
                popup.set_status_failed(failed.into());
                popup.set_service_unavailable(false);
            }
            if let Some(settings) = settings {
                let separator = app::digit_separator(with_app(|app| app.settings.general.language));
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
            row.entries = group_digits(*entries, separator).into();
        }
        VolumeState::Failed { reason } => {
            row.state = 2;
            row.reason = reason.as_str().into();
        }
    }
    row
}

fn to_result_row(row: &Row, icons: &IconLoader) -> ResultRow {
    match row {
        Row::Header(category) => ResultRow {
            header: true,
            category: category.index(),
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
                path: path.as_str().into(),
                is_dir: *is_dir,
                is_app: *is_app,
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
        move |_, event| {
            if matches!(event, WindowEvent::Focused(false)) {
                let _ = window.upgrade_in_event_loop(|window| {
                    if window.window().is_visible()
                        && !window.get_renaming()
                        && !window.get_editing()
                    {
                        hide_history(&window);
                    }
                });
            }
            EventResult::Propagate
        }
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
        let clipboard = &app.settings.clipboard;
        (
            app.settings.general.popup_monitor,
            clipboard.paused,
            clipboard.on_pick == OnPick::CopyAndPaste,
        )
    });
    window.invoke_reset();
    window.set_paused(paused);
    window.set_paste_by_default(paste);
    window.set_unavailable(with_history(|history| history.unavailable.clone()).into());
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

/// Asks for the next page when the list is a page that may continue and none is on its way.
fn load_more(window: &HistoryWindow) {
    let filter = history_filter(window);
    with_history(|history| {
        if history.search.is_some() || !history.more || history.loading_more {
            return;
        }
        if let Some(before) = history.list.last() {
            history.loading_more = true;
            history.client.send(ClipboardCommand::Page {
                filter,
                before: Some(before),
            });
        }
    });
}

fn clipboard_event(windows: &Windows, event: ClipboardEvent) {
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
                    append_rows(&window, &rows);
                }
            } else {
                let more = rows.len() == PAGE_SIZE;
                replace_rows(&window, &rows, more);
            }
        }
        ClipboardEvent::SearchResults { generation, rows } => {
            if visible && with_history(|history| history.search == Some(generation)) {
                replace_rows(&window, &rows, false);
            }
        }
        ClipboardEvent::Added(summary) => {
            if visible {
                row_added(&window, &summary);
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

fn append_rows(window: &HistoryWindow, rows: &[Summary]) {
    let model = history_rows(rows);
    with_history(|history| {
        history.list.append(rows.iter().map(|row| row.id).collect());
        for row in model {
            history.rows.push(row);
        }
        history.more = rows.len() == PAGE_SIZE;
        history.loading_more = false;
    });
    sync_selection(window);
}

/// A new copy goes on top when the list shows the newest rows and the filter takes it.
fn row_added(window: &HistoryWindow, summary: &Summary) {
    let accepted = match history_filter(window) {
        HistoryFilter::All => true,
        HistoryFilter::Favorites => summary.favorite,
        HistoryFilter::Text => summary.kind == Kind::Text,
        HistoryFilter::Images => summary.kind == Kind::Image,
    };
    if !accepted || with_history(|history| history.search.is_some()) {
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
    if changed {
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

fn content_arrived(window: &HistoryWindow, visible: bool, id: i64, content: Option<Content>) {
    let paste = with_history(|history| {
        history.requested = None;
        if !visible {
            return None;
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
