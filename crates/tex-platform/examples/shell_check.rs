//! Development aid: drives tex-platform's GNOME path against the `fake_shell` example, without
//! GNOME and without a display. Usage, with `fake_shell --press` running:
//! DISPLAY= XDG_CURRENT_DESKTOP=GNOME cargo run -p tex-platform --example shell_check

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::mpsc;
    use std::time::Duration;

    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use tex_platform::{Accelerator, HotkeyEvent, HotkeySlot, Hotkeys};

    let (active, extension) = mpsc::channel();
    tex_platform::watch_shell_extension(move |on| {
        let _ = active.send(on);
    });
    let on = extension
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or(false);
    println!("extension active: {on}");
    let (events, received) = mpsc::channel();
    let mut hotkeys = Hotkeys::start(move |event| {
        let _ = events.send(event);
    })
    .expect("hotkeys");
    println!("support: {:?}", hotkeys.support());
    let accelerator = Accelerator {
        hotkey: HotKey::new(Some(Modifiers::ALT), Code::Space),
        keysym: 0x20,
        mods: 8,
        gnome: "<Alt>space".into(),
        portal: "ALT+space".into(),
        label: "Open Tex".into(),
    };
    println!(
        "register: {:?}",
        hotkeys
            .register(HotkeySlot::Tex, &accelerator)
            .map_err(|error| error.to_string())
    );
    while let Ok(event) = received.recv_timeout(Duration::from_secs(5)) {
        println!("event: {event:?}");
        if event == HotkeyEvent::Pressed(HotkeySlot::Tex) {
            break;
        }
    }
    println!("work area: {:?}", tex_platform::work_area_under_cursor());
    println!(
        "present: {}",
        tex_platform::present_window("Tex", Some((610, 156)))
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {}
