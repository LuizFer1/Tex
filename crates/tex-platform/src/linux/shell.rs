use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::thread;

use zbus::MatchRule;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::names::BusName;

use super::dbus::session_bus;
use crate::{HotkeySlot, WorkArea};

pub(crate) const NAME: &str = "io.github.LuizFer1.Tex.Shell";
const PATH: &str = "/io/github/LuizFer1/Tex/Shell";
const INTERFACE: &str = "io.github.LuizFer1.Tex.Shell";
pub(crate) const UUID: &str = "tex@luizfer1.github.io";

type Listener = Box<dyn Fn(bool) + Send>;

static PRESENT: AtomicBool = AtomicBool::new(false);
static KNOWN: AtomicBool = AtomicBool::new(false);
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());
static FOLLOW: Once = Once::new();

/// Whether the session is GNOME, from `XDG_CURRENT_DESKTOP`.
pub(crate) fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|value| lists_gnome(&value))
}

/// `XDG_CURRENT_DESKTOP` is a list such as `ubuntu:GNOME`.
pub(crate) fn lists_gnome(value: &str) -> bool {
    value
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("GNOME"))
}

/// Whether Tex's GNOME Shell extension is on the session bus, as last seen.
pub(crate) fn present() -> bool {
    PRESENT.load(Ordering::SeqCst)
}

/// Calls `listener` with whether the extension is on the bus, at once when that is known
/// and then on every change, from the thread that follows the bus.
pub(crate) fn watch(listener: impl Fn(bool) + Send + 'static) {
    let mut listeners = LISTENERS.lock().unwrap();
    if KNOWN.load(Ordering::SeqCst) {
        listener(present());
    }
    listeners.push(Box::new(listener));
    drop(listeners);
    FOLLOW.call_once(|| {
        thread::spawn(follow);
    });
}

fn follow() {
    let Some(bus) = session_bus() else {
        return;
    };
    let Ok(changes) = owner_changes(&bus) else {
        return;
    };
    set_present(has_owner(&bus));
    for message in changes.flatten() {
        if let Ok((_, _, owner)) = message.body().deserialize::<(String, String, String)>() {
            set_present(!owner.is_empty());
        }
    }
}

fn has_owner(bus: &Connection) -> bool {
    let (Ok(dbus), Ok(name)) = (DBusProxy::new(bus), BusName::try_from(NAME)) else {
        return false;
    };
    dbus.name_has_owner(name).unwrap_or(false)
}

fn owner_changes(bus: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(NAME)?
        .build();
    MessageIterator::for_match_rule(rule, bus, None)
}

fn set_present(present: bool) {
    let listeners = LISTENERS.lock().unwrap();
    let first = !KNOWN.swap(true, Ordering::SeqCst);
    if PRESENT.swap(present, Ordering::SeqCst) != present || first {
        for listener in listeners.iter() {
            listener(present);
        }
    }
}

fn shell() -> Option<Proxy<'static>> {
    Proxy::new(&session_bus()?, NAME, PATH, INTERFACE).ok()
}

/// The work area of the monitor under the pointer, or of the primary one, as the extension
/// measures it. `None` without the extension.
pub(crate) fn work_area(primary: bool) -> Option<WorkArea> {
    if !present() {
        return None;
    }
    let method = if primary {
        "PrimaryWorkArea"
    } else {
        "WorkAreaUnderCursor"
    };
    let (x, y, width, height, scale): (i32, i32, u32, u32, f64) =
        shell()?.call(method, &()).ok()?;
    Some(WorkArea {
        x,
        y,
        width,
        height,
        scale: scale as f32,
    })
}

/// Asks the extension to move Tex's window titled `title` to `(x, y)` (or leave it where it is
/// when `x` is negative) and to activate it.
pub(crate) fn present_window(title: &str, x: i32, y: i32) -> bool {
    present() && shell().is_some_and(|shell| shell.call_method("Present", &(title, x, y)).is_ok())
}

/// Hands both GNOME accelerators to the extension (empty for none) and says which it could
/// grab. `None` without the extension.
pub(crate) fn set_shortcuts(tex: &str, history: &str) -> Option<(bool, bool)> {
    if !present() {
        return None;
    }
    shell()?.call("SetShortcuts", &(tex, history)).ok()
}

/// Calls `on_press`, from a thread of its own, whenever the extension reports a shortcut.
pub(crate) fn watch_shortcuts(on_press: impl Fn(HotkeySlot) + Send + 'static) {
    thread::spawn(move || {
        let Some(bus) = session_bus() else {
            return;
        };
        let Ok(signals) = shortcut_signals(&bus) else {
            return;
        };
        for message in signals.flatten() {
            match message.body().deserialize::<String>().as_deref() {
                Ok("tex") => on_press(HotkeySlot::Tex),
                Ok("history") => on_press(HotkeySlot::History),
                _ => {}
            }
        }
    });
}

fn shortcut_signals(bus: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(NAME)?
        .interface(INTERFACE)?
        .member("ShortcutActivated")?
        .build();
    MessageIterator::for_match_rule(rule, bus, None)
}

/// Turns the extension on, on a thread: through GNOME Shell when it already knows it, and in
/// `enabled-extensions` so a new session loads it when it was installed during this one.
pub(crate) fn enable() {
    thread::spawn(|| {
        if let Some(bus) = session_bus()
            && let Ok(extensions) = Proxy::new(
                &bus,
                "org.gnome.Shell.Extensions",
                "/org/gnome/Shell/Extensions",
                "org.gnome.Shell.Extensions",
            )
        {
            let _ = extensions.call::<_, _, bool>("EnableExtension", &(UUID,));
        }
        let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.shell", "enabled-extensions"])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        if let Some(value) = with_extension(&String::from_utf8_lossy(&output.stdout), UUID) {
            let _ = Command::new("gsettings")
                .args(["set", "org.gnome.shell", "enabled-extensions", &value])
                .status();
        }
    });
}

/// The `enabled-extensions` list as gsettings prints it, with `uuid` added. `None` when it is
/// already there or the value is not a list of strings.
pub(crate) fn with_extension(value: &str, uuid: &str) -> Option<String> {
    let value = value.trim();
    let list = value.strip_prefix("@as").unwrap_or(value).trim();
    let inner = list.strip_prefix('[')?.strip_suffix(']')?;
    let mut items: Vec<&str> = inner
        .split(',')
        .map(|item| item.trim().trim_matches(['\'', '"']))
        .filter(|item| !item.is_empty())
        .collect();
    if items.contains(&uuid) {
        return None;
    }
    items.push(uuid);
    let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    Some(format!("[{}]", quoted.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_is_found_in_the_desktop_list() {
        assert!(lists_gnome("GNOME"));
        assert!(lists_gnome("ubuntu:GNOME"));
        assert!(lists_gnome("gnome"));
        assert!(!lists_gnome("KDE"));
        assert!(!lists_gnome("X-Cinnamon"));
        assert!(!lists_gnome(""));
    }

    #[test]
    fn the_extension_is_added_to_the_enabled_list() {
        assert_eq!(
            with_extension("@as []\n", UUID).as_deref(),
            Some("['tex@luizfer1.github.io']")
        );
        assert_eq!(
            with_extension("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com']\n", UUID).as_deref(),
            Some("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com', 'tex@luizfer1.github.io']")
        );
    }

    #[test]
    fn an_enabled_extension_or_a_strange_value_is_left_alone() {
        assert_eq!(
            with_extension("['tex@luizfer1.github.io', 'a@b']", UUID),
            None
        );
        assert_eq!(with_extension("nothing", UUID), None);
        assert_eq!(with_extension("", UUID), None);
    }
}
