#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon;
mod placement;
#[allow(dead_code)]
mod results;

use std::error::Error;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, PhysicalPosition};
use tex_platform::{SingleInstance, work_area_under_cursor};

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

    let hotkeys = GlobalHotKeyManager::new()?;
    if let Err(error) = hotkeys.register(HotKey::new(Some(Modifiers::ALT), Code::Space)) {
        eprintln!("Alt+Space is not available, use the tray icon instead: {error}");
    }
    GlobalHotKeyEvent::set_event_handler(Some({
        let popup = popup.as_weak();
        move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = popup.upgrade_in_event_loop(|popup| toggle_popup(&popup));
            }
        }
    }));

    tray.on_open({
        let popup = popup.as_weak();
        move || {
            if let Some(popup) = popup.upgrade() {
                show_popup(&popup);
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

fn toggle_popup(popup: &Popup) {
    if popup.window().is_visible() {
        let _ = popup.hide();
    } else {
        show_popup(popup);
    }
}

/// Clears the query, moves the popup to the monitor under the cursor, and gives it keyboard focus.
fn show_popup(popup: &Popup) {
    popup.invoke_reset();
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
