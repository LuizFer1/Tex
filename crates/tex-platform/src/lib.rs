#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    SingleInstance, cache_dir, config_dir, data_dir, local_time, lock_foreground, open_locked,
    open_path, primary_work_area, restore_foreground, reveal_in_folder, round_corners,
    shape_tabbed_window, system_uses_dark_theme, take_foreground, user_language,
    work_area_under_cursor,
};
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
pub use clipboard::{ClipboardWatcher, foreground_window, paste_into, write_image, write_text};
#[cfg(windows)]
mod dpapi;
#[cfg(windows)]
pub use dpapi::{protect, unprotect};
#[cfg(windows)]
mod http;
#[cfg(windows)]
pub use http::http_get;
#[cfg(windows)]
mod file;
#[cfg(windows)]
pub use file::pick_file;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
pub use icon::{app_icon, init_icon_thread};
#[cfg(windows)]
mod folder;
#[cfg(windows)]
pub use folder::pick_folder;
#[cfg(windows)]
mod winkey;
#[cfg(windows)]
pub use winkey::{WinKeyHook, catch_windows_key};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    ClipboardWatcher, SingleInstance, WinKeyHook, app_icon, cache_dir, catch_windows_key,
    config_dir, data_dir, foreground_window, http_get, init_icon_thread, local_time,
    lock_foreground, open_locked, open_path, paste_into, pick_file, pick_folder, primary_work_area,
    protect, restore_foreground, reveal_in_folder, round_corners, shape_tabbed_window,
    system_uses_dark_theme, take_foreground, unprotect, user_language, work_area_under_cursor,
    write_image, write_text,
};

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("tex-platform only supports Windows and Linux");

/// Usable area of a monitor (screen minus taskbar) in physical pixels, with its DPI scale (1.0 = 96 DPI).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// A wall-clock time in the user's time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Straight (not premultiplied) RGBA pixels, row by row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What the clipboard held after one update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clip {
    pub text: Option<String>,
    /// `CF_DIBV5`, or `CF_DIB` when there is no V5 header.
    pub dib: Option<Vec<u8>>,
    /// `ExcludeClipboardContentFromMonitorProcessing` is present.
    pub excluded: bool,
    /// `CanIncludeInClipboardHistory` is present with the value 0.
    pub history_disallowed: bool,
    /// Written by Tex itself; text and image are not read.
    pub from_tex: bool,
    /// File name of the owner's executable, such as `notepad.exe`.
    pub owner_exe: Option<String>,
}

/// The formats on the clipboard, known before any content is read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipFormats {
    /// `CF_UNICODETEXT` is offered and, once read, not empty.
    pub text: bool,
    /// `CF_DIBV5` or `CF_DIB` is offered.
    pub image: bool,
    pub excluded: bool,
    pub history_disallowed: bool,
    pub from_tex: bool,
    pub owner_exe: Option<String>,
}

/// The content worth reading for a [`ClipFormats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    Text,
    Image,
}
