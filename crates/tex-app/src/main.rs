#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clipboard_client;
mod icon;
mod icons;
mod placement;
mod results;
mod search_client;
mod settings;
mod shortcut;

use std::cell::RefCell;
use std::error::Error;
use std::time::Instant;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, PhysicalPosition, Rgba8Pixel,
    SharedPixelBuffer, StandardListViewItem, VecModel, Weak,
};
use tex_ipc::{Filter, VolumeState, VolumeStatus};
use tex_platform::{
    ClipboardWatcher, IconPixels, SingleInstance, open_path, pick_folder, primary_work_area,
    reveal_in_folder, work_area_under_cursor,
};

use crate::app::{AppState, HotkeyError, with_app};
use crate::clipboard_client::{ClipboardClient, history_folder};
use crate::icons::{IconCache, IconLoader, IconState};
use crate::results::{Row, build_rows, group_digits, status_parts, step_selection};
use crate::search_client::{Command, Event, SearchClient};
use crate::settings::{
    EnterAction, KeepSearch, Language, PopupMonitor, Settings, Theme, settings_path,
};
use crate::shortcut::Shortcut;

slint::include_modules!();

const ICON_CACHE_SIZE: usize = 256;

thread_local! {
    static ICONS: RefCell<IconCache<Image>> = RefCell::new(IconCache::new(ICON_CACHE_SIZE));
}

/// Weak handles to every window, cheap to clone into callbacks and across threads.
#[derive(Clone)]
struct Windows {
    popup: Weak<Popup>,
    about: Weak<AboutWindow>,
    settings: Weak<SettingsWindow>,
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
    let tray = TexTray::new()?;
    about.set_version(env!("CARGO_PKG_VERSION").into());
    tray.set_icon_image(icon::tray_icon());
    let windows = Windows {
        popup: popup.as_weak(),
        about: about.as_weak(),
        settings: settings_window.as_weak(),
    };

    app::init(AppState::new(settings, path, GlobalHotKeyManager::new()?));
    let language = with_app(|app| app.settings.general.language);
    app::apply_language(language);
    apply_theme(&windows);
    with_app(|app| apply_categories(&popup, &app.settings));

    let clipboard = ClipboardClient::start(
        history_folder(),
        with_app(|app| app::client_settings(&app.settings.clipboard)),
        |event| {
            let _ = slint::invoke_from_event_loop(move || clipboard_event(event));
        },
    );
    let _watcher = match ClipboardWatcher::start({
        let clipboard = clipboard.clone();
        move |clip| clipboard.send(clipboard_client::Command::Captured(clip))
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

    with_app(|app| app.resume_hotkey());
    GlobalHotKeyEvent::set_event_handler(Some({
        let windows = windows.clone();
        let client = client.clone();
        move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                open_from_key(&windows, &client);
            }
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

    tray.on_open({
        let windows = windows.clone();
        let client = client.clone();
        move || show_popup(&windows, &client)
    });
    tray.on_settings({
        let windows = windows.clone();
        let client = client.clone();
        move || show_settings(&windows, &client)
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

/// The history window handles these in a later task.
fn clipboard_event(event: clipboard_client::Event) {
    use clipboard_client::Event;
    match event {
        Event::Unavailable(_)
        | Event::Page { .. }
        | Event::SearchResults { .. }
        | Event::Added(_)
        | Event::Changed(_)
        | Event::Removed(_)
        | Event::Content(..)
        | Event::EditRefused(..)
        | Event::Usage(_) => {}
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
            show_settings(&windows, &client);
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

fn show_settings(windows: &Windows, client: &SearchClient) {
    let Some(window) = windows.settings.upgrade() else {
        return;
    };
    apply_theme(windows);
    with_app(|app| fill_settings(&window, app));
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
