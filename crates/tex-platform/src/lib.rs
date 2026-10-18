#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    AppBus, SingleInstance, cache_dir, can_place_windows, config_dir, data_dir,
    enable_shell_extension, is_gnome, local_time, lock_foreground, open_locked, open_path,
    present_window, primary_work_area, restore_foreground, reveal_in_folder, round_corners,
    send_command, shape_tabbed_window, system_uses_dark_theme, take_foreground,
    transparent_windows, user_language, watch_shell_extension, watch_theme, work_area_under_cursor,
};
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
pub use hotkeys::Hotkeys;
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
    AppBus, ClipboardWatcher, Hotkeys, SingleInstance, WinKeyHook, app_icon, cache_dir,
    can_place_windows, catch_windows_key, config_dir, data_dir, enable_shell_extension,
    foreground_window, http_get, init_icon_thread, is_gnome, local_time, lock_foreground,
    open_locked, open_path, paste_into, pick_file, pick_folder, present_window, primary_work_area,
    protect, restore_foreground, reveal_in_folder, round_corners, send_command,
    shape_tabbed_window, skip_taskbar, system_uses_dark_theme, take_foreground,
    transparent_windows, unprotect, user_language, watch_shell_extension, watch_theme,
    work_area_under_cursor, write_image, write_text,
};

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("tex-platform only supports Windows and Linux");

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// A top-level window the platform can address: the HWND on Windows, the X11 window id on
/// Linux. Wayland windows have none, since only the compositor moves and focuses them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowRef(pub(crate) isize);

impl WindowRef {
    /// The window behind `window` (a winit window, for instance), when the platform can address it.
    pub fn of(window: &impl HasWindowHandle) -> Option<Self> {
        match window.window_handle().ok()?.as_raw() {
            #[cfg(windows)]
            RawWindowHandle::Win32(handle) => Some(Self(handle.hwnd.get())),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xlib(handle) => Some(Self(handle.window as isize)),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xcb(handle) => Some(Self(handle.window.get() as isize)),
            _ => None,
        }
    }
}

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

/// What the running Tex is asked to do by another Tex (`tex --toggle`), the GNOME extension
/// or any other program on the session bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    Toggle,
    History,
    Settings,
    About,
    TogglePause,
    ApplyUpdate,
    Quit,
}

/// One of the two global shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySlot {
    Tex,
    History,
}

impl HotkeySlot {
    pub(crate) fn index(self) -> usize {
        match self {
            HotkeySlot::Tex => 0,
            HotkeySlot::History => 1,
        }
    }
}

/// A shortcut in the form each backend reads, built by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accelerator {
    /// For `global-hotkey` on Windows.
    pub hotkey: global_hotkey::hotkey::HotKey,
    /// X11 keysym of the key.
    pub keysym: u32,
    /// X11 modifier mask: Shift 1, Control 4, Mod1 (Alt) 8, Mod4 (Super) 64.
    pub mods: u16,
    /// GNOME Shell accelerator, such as `<Alt>space`.
    pub gnome: String,
    /// Trigger for the GlobalShortcuts portal, such as `ALT+space`.
    pub portal: String,
    /// What the shortcut does, in the user's language; the portal shows it.
    pub label: String,
}

/// How global shortcuts work in this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySupport {
    /// Tex registers the keys from its settings (Windows, X11, GNOME with the extension).
    Free,
    /// The desktop picks the keys and may ask the user first (the GlobalShortcuts portal).
    System,
    /// Nothing can register a global shortcut here.
    Unavailable,
}

/// What the shortcut backend reports, on one of its threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed(HotkeySlot),
    /// The portal bound the shortcuts: the keys of each as the desktop describes them,
    /// `None` when it refused one.
    Bound {
        tex: Option<String>,
        history: Option<String>,
    },
    /// The support changed (the GNOME extension came or went): register the shortcuts again.
    Changed,
}

/// The content worth reading for a [`ClipFormats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    Text,
    Image,
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{HandleError, WindowHandle};

    struct Fake(RawWindowHandle);

    impl HasWindowHandle for Fake {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Ok(unsafe { WindowHandle::borrow_raw(self.0) })
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_win32_window_is_its_hwnd() {
        let hwnd = std::num::NonZeroIsize::new(0x1234).unwrap();
        let raw = RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(hwnd));
        assert_eq!(WindowRef::of(&Fake(raw)), Some(WindowRef(0x1234)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_x11_window_is_its_id() {
        let xlib = RawWindowHandle::Xlib(raw_window_handle::XlibWindowHandle::new(0x2a));
        assert_eq!(WindowRef::of(&Fake(xlib)), Some(WindowRef(0x2a)));
        let id = std::num::NonZeroU32::new(0x2b).unwrap();
        let xcb = RawWindowHandle::Xcb(raw_window_handle::XcbWindowHandle::new(id));
        assert_eq!(WindowRef::of(&Fake(xcb)), Some(WindowRef(0x2b)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_wayland_window_has_no_ref() {
        let surface = std::ptr::NonNull::<std::ffi::c_void>::dangling();
        let raw = RawWindowHandle::Wayland(raw_window_handle::WaylandWindowHandle::new(surface));
        assert_eq!(WindowRef::of(&Fake(raw)), None);
    }
}
