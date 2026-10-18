use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy, connection};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub(crate) const DESKTOP: &str = "org.freedesktop.portal.Desktop";
pub(crate) const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";

static DARK: AtomicBool = AtomicBool::new(false);

/// The session bus, connected once. `None` without one (a bare X session, for instance).
pub(crate) fn session_bus() -> Option<Connection> {
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

/// Asks the portal's file chooser for one file, or one folder when `directory` is set, and
/// waits for the answer. `None` when cancelled or without a portal.
pub(crate) fn choose(title: &str, directory: bool) -> Option<String> {
    let bus = session_bus()?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .ok()?
        .member("Response")
        .ok()?
        .build();
    let responses = MessageIterator::for_match_rule(rule, &bus, None).ok()?;
    let chooser = Proxy::new(
        &bus,
        DESKTOP,
        DESKTOP_PATH,
        "org.freedesktop.portal.FileChooser",
    )
    .ok()?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("directory", Value::from(directory));
    let request: OwnedObjectPath = chooser.call("OpenFile", &("", title, options)).ok()?;
    for message in responses.flatten() {
        let header = message.header();
        if header.path().map(|path| path.as_str()) != Some(request.as_str()) {
            continue;
        }
        let (response, mut results) = message
            .body()
            .deserialize::<(u32, HashMap<String, OwnedValue>)>()
            .ok()?;
        if response != 0 {
            return None;
        }
        let uris = Vec::<String>::try_from(results.remove("uris")?).ok()?;
        return uris.first().and_then(|uri| uri_to_path(uri));
    }
    None
}

/// `color-scheme` 1 means "prefer dark"; 0 (no preference) and 2 (prefer light) are light.
/// The older `Read` wraps the number in one more variant.
pub(crate) fn is_dark(value: &Value<'_>) -> bool {
    matches!(value.downcast_ref::<u32>(), Ok(1))
}

/// Opens the file manager on the folder of `path` with it selected.
pub(crate) fn show_item(path: &str) -> zbus::Result<()> {
    let bus = session_bus().ok_or_else(|| zbus::Error::Failure("no session bus".into()))?;
    let manager = Proxy::new(
        &bus,
        "org.freedesktop.FileManager1",
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1",
    )?;
    manager
        .call_method("ShowItems", &(vec![file_uri(path)], ""))
        .map(drop)
}

/// The local path of a `file://` URI, percent-decoded. `None` for other schemes.
pub(crate) fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let raw = &rest.as_bytes()[rest.find('/')?..];
    let hex = |byte: Option<&u8>| byte.and_then(|byte| (*byte as char).to_digit(16));
    let mut bytes = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let decoded = (raw[index] == b'%')
            .then(|| Some(hex(raw.get(index + 1))? * 16 + hex(raw.get(index + 2))?))
            .flatten();
        match decoded {
            Some(byte) => {
                bytes.push(byte as u8);
                index += 3;
            }
            None => {
                bytes.push(raw[index]);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// A `file://` URI for an absolute path: every byte but letters, digits, `-._~` and `/` is
/// percent-encoded.
pub(crate) fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
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

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(
            uri_to_path("file:///home/ana/My%20File.txt").as_deref(),
            Some("/home/ana/My File.txt")
        );
        assert_eq!(
            uri_to_path("file://localhost/tmp/a%C3%A7").as_deref(),
            Some("/tmp/aç")
        );
        assert_eq!(uri_to_path("file:///bad%zz").as_deref(), Some("/bad%zz"));
        assert_eq!(uri_to_path("https://example.com/a"), None);
        assert_eq!(uri_to_path("file:///x%FF"), None);
    }

    #[test]
    fn paths_become_file_uris() {
        assert_eq!(
            file_uri("/home/ana/My File#1.txt"),
            "file:///home/ana/My%20File%231.txt"
        );
        assert_eq!(file_uri("/tmp/aç"), "file:///tmp/a%C3%A7");
        let path = "/home/ana/100% [done]/ação.txt";
        assert_eq!(uri_to_path(&file_uri(path)).as_deref(), Some(path));
    }
}
