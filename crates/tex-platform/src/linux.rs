use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use crate::{Clip, ClipFormats, IconPixels, LocalTime, Payload, WindowRef, WorkArea};

/// Held while this process is the only Tex of the user; the lock goes away with the process.
pub struct SingleInstance {
    _lock: File,
}

impl SingleInstance {
    /// `None` when another process already holds the lock named after `name`.
    pub fn acquire(name: &str) -> io::Result<Option<Self>> {
        let folder = runtime_dir();
        fs::create_dir_all(&folder)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder.join(lock_file_name(name)))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(Self { _lock: lock })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(folder) if folder.is_absolute() => folder.join("tex"),
        _ => std::env::temp_dir().join(format!("tex-{}", unsafe { libc::getuid() })),
    }
}

pub(crate) fn lock_file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{clean}.lock")
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("tex")
}

pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("tex")
}

pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join("tex")
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    xdg_base(
        std::env::var_os(variable),
        std::env::var_os("HOME"),
        fallback,
    )
}

/// The XDG base folder: the variable when it is an absolute path, else `fallback` under home.
pub(crate) fn xdg_base(value: Option<OsString>, home: Option<OsString>, fallback: &str) -> PathBuf {
    match value.map(PathBuf::from) {
        Some(folder) if folder.is_absolute() => folder,
        _ => PathBuf::from(home.unwrap_or_default()).join(fallback),
    }
}

/// The user's language as a tag such as "pt-BR", from the locale variables. Empty if none is set.
pub fn user_language() -> String {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find_map(|value| language_tag(&value))
        .unwrap_or_default()
}

pub(crate) fn language_tag(value: &str) -> Option<String> {
    let first = value.split(':').next()?;
    let base = first.split(['.', '@']).next()?;
    (!base.is_empty() && base != "C" && base != "POSIX").then(|| base.replace('_', "-"))
}

/// Converts a Unix time in milliseconds to the user's local time.
pub fn local_time(unix_ms: i64) -> LocalTime {
    let seconds = unix_ms.div_euclid(1000) as libc::time_t;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut parts) };
    LocalTime {
        year: (parts.tm_year + 1900) as u16,
        month: (parts.tm_mon + 1) as u8,
        day: parts.tm_mday as u8,
        hour: parts.tm_hour as u8,
        minute: parts.tm_min as u8,
    }
}

pub fn open_path(path: &str) -> io::Result<()> {
    spawn_detached("xdg-open", path)
}

pub fn reveal_in_folder(path: &str) -> io::Result<()> {
    let folder = Path::new(path).parent().map_or_else(
        || path.to_owned(),
        |parent| parent.to_string_lossy().into_owned(),
    );
    spawn_detached("xdg-open", &folder)
}

fn spawn_detached(program: &str, argument: &str) -> io::Result<()> {
    let mut child = Command::new(program).arg(argument).spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn open_locked(path: &Path) -> io::Result<File> {
    File::open(path)
}

pub fn system_uses_dark_theme() -> bool {
    false
}

pub fn work_area_under_cursor() -> Option<WorkArea> {
    None
}

pub fn primary_work_area() -> Option<WorkArea> {
    None
}

pub fn shape_tabbed_window(
    _window: WindowRef,
    _width: i32,
    _height: i32,
    _tab_width: i32,
    _tab_height: i32,
    _radius: i32,
) {
}

pub fn round_corners(_window: WindowRef) {}

pub fn lock_foreground(_lock: bool) {}

pub fn take_foreground(_window: WindowRef) -> bool {
    false
}

pub fn restore_foreground(_window: WindowRef) {}

pub fn foreground_window() -> WindowRef {
    WindowRef::default()
}

pub fn paste_into(_window: WindowRef) {}

pub struct ClipboardWatcher;

impl ClipboardWatcher {
    pub fn start(
        _wanted: impl Fn(&ClipFormats) -> Option<Payload> + Send + 'static,
        _on_change: impl Fn(Clip) + Send + 'static,
    ) -> io::Result<ClipboardWatcher> {
        Err(unsupported())
    }
}

pub fn write_text(_text: &str) -> io::Result<()> {
    Err(unsupported())
}

pub fn write_image(_dib: &[u8], _png: &[u8]) -> io::Result<()> {
    Err(unsupported())
}

pub fn protect(_data: &[u8]) -> io::Result<Vec<u8>> {
    Err(unsupported())
}

pub fn unprotect(_data: &[u8]) -> io::Result<Vec<u8>> {
    Err(unsupported())
}

pub fn http_get(_url: &str, _max_bytes: u64) -> io::Result<Vec<u8>> {
    Err(unsupported())
}

pub fn pick_file(
    _title: &str,
    _filter_name: &str,
    on_done: impl FnOnce(Option<String>) + Send + 'static,
) {
    thread::spawn(move || on_done(None));
}

pub fn pick_folder(_title: &str, on_done: impl FnOnce(Option<String>) + Send + 'static) {
    thread::spawn(move || on_done(None));
}

pub fn init_icon_thread() {}

pub fn app_icon(_path: &str) -> Option<IconPixels> {
    None
}

pub struct WinKeyHook;

impl WinKeyHook {
    pub fn start(_on_press: impl Fn() + Send + 'static) -> io::Result<WinKeyHook> {
        Err(unsupported())
    }
}

pub fn catch_windows_key(_window: WindowRef, _on_press: impl Fn() -> bool + 'static) {}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "not available on Linux yet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_names_keep_only_safe_characters() {
        assert_eq!(
            lock_file_name(r"Local\Tex.SingleInstance"),
            "Local-Tex.SingleInstance.lock"
        );
    }

    #[test]
    fn a_second_lock_on_the_same_name_is_refused() {
        let name = format!("tex-test-{}", std::process::id());
        let first = SingleInstance::acquire(&name).unwrap();
        assert!(first.is_some());
        assert!(SingleInstance::acquire(&name).unwrap().is_none());
        drop(first);
        assert!(SingleInstance::acquire(&name).unwrap().is_some());
    }

    #[test]
    fn locale_values_become_language_tags() {
        assert_eq!(language_tag("pt_BR.UTF-8").as_deref(), Some("pt-BR"));
        assert_eq!(language_tag("en_US@euro").as_deref(), Some("en-US"));
        assert_eq!(language_tag("pt_BR:pt:en").as_deref(), Some("pt-BR"));
        assert_eq!(language_tag("C.UTF-8"), None);
        assert_eq!(language_tag("POSIX"), None);
        assert_eq!(language_tag(""), None);
    }

    #[test]
    fn xdg_folders_ignore_relative_values() {
        let home = Some(OsString::from("/home/ana"));
        assert_eq!(
            xdg_base(Some("/data".into()), home.clone(), ".local/share"),
            PathBuf::from("/data")
        );
        assert_eq!(
            xdg_base(Some("relative".into()), home.clone(), ".local/share"),
            PathBuf::from("/home/ana/.local/share")
        );
        assert_eq!(
            xdg_base(None, home, ".config"),
            PathBuf::from("/home/ana/.config")
        );
    }
}
