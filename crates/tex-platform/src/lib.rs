#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    LocalTime, SingleInstance, local_time, lock_foreground, open_path, primary_work_area,
    restore_foreground, reveal_in_folder, system_uses_dark_theme, take_foreground, user_language,
    work_area_under_cursor,
};
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
pub use clipboard::{
    Clip, ClipFormats, ClipboardWatcher, Payload, foreground_window, paste_into, write_image,
    write_text,
};
#[cfg(windows)]
mod dpapi;
#[cfg(windows)]
pub use dpapi::{protect, unprotect};
#[cfg(windows)]
mod file;
#[cfg(windows)]
pub use file::pick_file;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
pub use icon::{IconPixels, app_icon, init_icon_thread};
#[cfg(windows)]
mod folder;
#[cfg(windows)]
pub use folder::pick_folder;
#[cfg(windows)]
mod winkey;
#[cfg(windows)]
pub use winkey::{WinKeyHook, catch_windows_key};

#[cfg(not(windows))]
compile_error!("tex-platform only supports Windows for now");

/// Usable area of a monitor (screen minus taskbar) in physical pixels, with its DPI scale (1.0 = 96 DPI).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}
