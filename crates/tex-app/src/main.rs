#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon;
#[allow(dead_code)]
mod icons;
mod placement;
mod results;
mod search_client;

use std::error::Error;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, VecModel};
use tex_ipc::Filter;
use tex_platform::{SingleInstance, open_path, reveal_in_folder, work_area_under_cursor};

use crate::results::{Row, SERVICE_UNAVAILABLE, build_rows, status_text, step_selection};
use crate::search_client::{Command, Event, SearchClient};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    let Some(_instance) = SingleInstance::acquire("Local\\Tex.SingleInstance")? else {
        return Ok(());
    };

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;

    let popup = Popup::new()?;
    let about = AboutWindow::new()?;
    let tray = TexTray::new()?;
    about.set_version(env!("CARGO_PKG_VERSION").into());
    tray.set_icon_image(icon::tray_icon());

    let client = SearchClient::start(std::env::var("USERPROFILE").unwrap_or_default(), {
        let popup = popup.as_weak();
        move |event| {
            let _ = popup.upgrade_in_event_loop(move |popup| show_event(&popup, event));
        }
    });

    popup.on_dismissed({
        let popup = popup.as_weak();
        move || {
            if let Some(popup) = popup.upgrade() {
                let _ = popup.hide();
            }
        }
    });
    popup.window().on_winit_window_event({
        let popup = popup.as_weak();
        move |_, event| {
            if matches!(event, WindowEvent::Focused(false)) {
                let _ = popup.upgrade_in_event_loop(|popup| {
                    let _ = popup.hide();
                });
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
        move |index, reveal| {
            if let Some(popup) = popup.upgrade() {
                activate(&popup, index, reveal);
            }
        }
    });

    let hotkeys = GlobalHotKeyManager::new()?;
    if let Err(error) = hotkeys.register(HotKey::new(Some(Modifiers::ALT), Code::Space)) {
        eprintln!("Alt+Space is not available, use the tray icon instead: {error}");
    }
    GlobalHotKeyEvent::set_event_handler(Some({
        let popup = popup.as_weak();
        let client = client.clone();
        move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let client = client.clone();
                let _ = popup.upgrade_in_event_loop(move |popup| toggle_popup(&popup, &client));
            }
        }
    }));

    tray.on_open({
        let popup = popup.as_weak();
        let client = client.clone();
        move || {
            if let Some(popup) = popup.upgrade() {
                show_popup(&popup, &client);
            }
        }
    });
    tray.on_about({
        let about = about.as_weak();
        move || {
            if let Some(about) = about.upgrade() {
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

fn toggle_popup(popup: &Popup, client: &SearchClient) {
    if popup.window().is_visible() {
        let _ = popup.hide();
    } else {
        show_popup(popup, client);
    }
}

/// Clears the last search, asks the service for its status, moves the popup to the monitor
/// under the cursor and gives it keyboard focus.
fn show_popup(popup: &Popup, client: &SearchClient) {
    popup.invoke_reset();
    popup.set_rows(ModelRc::default());
    popup.set_selected(-1);
    popup.set_status_text("".into());
    client.send(Command::Status);
    if let Some(area) = work_area_under_cursor() {
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
    });
}

fn move_selection(popup: &Popup, step: i32) {
    let is_item: Vec<bool> = popup.get_rows().iter().map(|row| !row.header).collect();
    let current = usize::try_from(popup.get_selected()).ok();
    if let Some(next) = step_selection(&is_item, current, step as isize) {
        popup.set_selected(next as i32);
    }
}

fn show_event(popup: &Popup, event: Event) {
    match event {
        Event::Results(results) => {
            if popup.get_query().trim().is_empty() {
                return;
            }
            let rows = build_rows(&results.hits);
            let is_item: Vec<bool> = rows.iter().map(Row::is_item).collect();
            let selected = step_selection(&is_item, None, 1);
            let model: Vec<ResultRow> = rows.iter().map(to_result_row).collect();
            popup.set_rows(ModelRc::new(VecModel::from(model)));
            popup.set_selected(selected.map_or(-1, |index| index as i32));
            if popup.get_status_text() == SERVICE_UNAVAILABLE {
                popup.set_status_text("".into());
            }
        }
        Event::Status(volumes) => popup.set_status_text(status_text(&volumes).into()),
        Event::Unavailable => {
            popup.set_rows(ModelRc::default());
            popup.set_selected(-1);
            popup.set_status_text(SERVICE_UNAVAILABLE.into());
        }
    }
}

fn to_result_row(row: &Row) -> ResultRow {
    match row {
        Row::Header(title) => ResultRow {
            header: true,
            title: (*title).into(),
            ..Default::default()
        },
        Row::Item {
            name,
            folder,
            path,
            is_dir,
            is_app,
        } => ResultRow {
            header: false,
            title: name.as_str().into(),
            folder: folder.as_str().into(),
            path: path.as_str().into(),
            is_dir: *is_dir,
            is_app: *is_app,
        },
    }
}

fn activate(popup: &Popup, index: i32, reveal: bool) {
    let Some(row) = usize::try_from(index)
        .ok()
        .and_then(|index| popup.get_rows().row_data(index))
    else {
        return;
    };
    if row.header {
        return;
    }
    let opened = if reveal {
        reveal_in_folder(&row.path)
    } else {
        open_path(&row.path)
    };
    if opened.is_ok() {
        let _ = popup.hide();
    }
}
