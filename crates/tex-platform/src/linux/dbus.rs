use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy, connection};
use zbus::message::Type;
use zbus::zvariant::{OwnedValue, Value};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";

static DARK: AtomicBool = AtomicBool::new(false);

/// The session bus, connected once. `None` without one (a bare X session, for instance).
fn session_bus() -> Option<Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        connection::Builder::session()
            .ok()?
            .method_timeout(Duration::from_secs(5))
            .build()
            .ok()
    })
    .clone()
}

pub(crate) fn system_uses_dark_theme() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Reads the portal's color scheme on a thread of its own, then follows its changes, calling
/// `on_change` whenever the dark preference flips. Without a portal the theme stays light.
pub(crate) fn watch_theme(on_change: impl Fn() + Send + 'static) {
    thread::spawn(move || {
        let Some(bus) = session_bus() else {
            return;
        };
        let Ok(changes) = setting_changes(&bus) else {
            return;
        };
        if let Ok(value) = read_color_scheme(&bus) {
            set_dark(is_dark(&value), &on_change);
        }
        for message in changes.flatten() {
            let Ok((namespace, key, value)) =
                message.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace == APPEARANCE && key == COLOR_SCHEME {
                set_dark(is_dark(&value), &on_change);
            }
        }
    });
}

fn setting_changes(bus: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.portal.Settings")?
        .member("SettingChanged")?
        .build();
    MessageIterator::for_match_rule(rule, bus, None)
}

/// `ReadOne` (portal 1.15+), else the older `Read`.
fn read_color_scheme(bus: &Connection) -> zbus::Result<OwnedValue> {
    let settings = Proxy::new(
        bus,
        DESKTOP,
        DESKTOP_PATH,
        "org.freedesktop.portal.Settings",
    )?;
    settings
        .call("ReadOne", &(APPEARANCE, COLOR_SCHEME))
        .or_else(|_| settings.call("Read", &(APPEARANCE, COLOR_SCHEME)))
}

fn set_dark(dark: bool, on_change: &impl Fn()) {
    if DARK.swap(dark, Ordering::Relaxed) != dark {
        on_change();
    }
}

/// `color-scheme` 1 means "prefer dark"; 0 (no preference) and 2 (prefer light) are light.
/// The older `Read` wraps the number in one more variant.
pub(crate) fn is_dark(value: &Value<'_>) -> bool {
    matches!(value.downcast_ref::<u32>(), Ok(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_prefer_dark_is_dark() {
        assert!(is_dark(&Value::from(1u32)));
        assert!(is_dark(&Value::new(Value::from(1u32))));
        assert!(!is_dark(&Value::from(0u32)));
        assert!(!is_dark(&Value::from(2u32)));
        assert!(!is_dark(&Value::from("dark")));
    }
}
