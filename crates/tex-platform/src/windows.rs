use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, HWND, POINT, SYSTEMTIME,
};
use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmGetWindowAttribute,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HMONITOR,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, RGN_OR, SetWindowRgn,
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::System::SystemServices::LOCALE_NAME_MAX_LENGTH;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetCursorPos, GetForegroundWindow, GetWindowLongW, IsIconic, IsWindow,
    IsWindowVisible, LSFW_LOCK, LSFW_UNLOCK, LockSetForegroundWindow, SW_SHOWNORMAL,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowLongW, SetWindowPos, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SIZEBOX,
    WS_SYSMENU,
};

use crate::winkey::{MASK_KEY, tap_key};
use crate::{LocalTime, WorkArea};

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// Takes a named mutex held until drop. Returns `Ok(None)` when another process already holds it.
    pub fn acquire(name: &str) -> io::Result<Option<Self>> {
        let wide = wide(name);
        let handle = unsafe { CreateMutexW(ptr::null(), 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Work area and DPI scale of the monitor under the mouse cursor, falling back to the primary monitor.
pub fn work_area_under_cursor() -> Option<WorkArea> {
    let mut cursor = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut cursor) };
    monitor_work_area(unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY) })
}

/// Clips a window to a tab on its top-left above a body whose other three corners are
/// rounded, so the area beside the tab is not part of the window. Physical pixels.
///
/// A window with a region is no longer drawn by DWM, so Windows would paint the classic
/// caption of a captioned window over its content; the caption styles are removed first.
/// winit sets them again whenever it shows the window, so call this after every show.
pub fn shape_tabbed_window(
    window: isize,
    width: i32,
    height: i32,
    tab_width: i32,
    tab_height: i32,
    radius: i32,
) {
    let diameter = radius * 2;
    unsafe {
        let style = GetWindowLongW(window as HWND, GWL_STYLE) as u32;
        let captionless = (style
            & !(WS_CAPTION | WS_SYSMENU | WS_SIZEBOX | WS_MAXIMIZEBOX | WS_MINIMIZEBOX))
            | WS_POPUP;
        if captionless != style {
            SetWindowLongW(window as HWND, GWL_STYLE, captionless as i32);
            SetWindowPos(
                window as HWND,
                ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let shape = CreateRoundRectRgn(0, tab_height, width + 1, height + 1, diameter, diameter);
        let corner = CreateRectRgn(0, tab_height, radius + 1, tab_height + radius + 1);
        let tab = CreateRoundRectRgn(
            0,
            0,
            tab_width + 1,
            tab_height + diameter,
            diameter,
            diameter,
        );
        CombineRgn(shape, shape, corner, RGN_OR);
        CombineRgn(shape, shape, tab, RGN_OR);
        DeleteObject(corner);
        DeleteObject(tab);
        if SetWindowRgn(window as HWND, shape, 1) == 0 {
            DeleteObject(shape);
        }
    }
}

/// Asks Windows 11 to round a frameless window's corners; older versions ignore it.
pub fn round_corners(window: isize) {
    let preference = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            window as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&preference as *const i32).cast(),
            size_of::<i32>() as u32,
        );
    }
}

/// While locked, other processes cannot take the foreground from this one; clicking another
/// window or pressing Alt still can. Only works while this process is in the foreground.
pub fn lock_foreground(lock: bool) {
    unsafe { LockSetForegroundWindow(if lock { LSFW_LOCK } else { LSFW_UNLOCK }) };
}

/// Brings `window`, a window of this process, to the foreground. Windows only lets the process
/// that got the last input take it, and a key the hook swallowed does not count, so when the
/// direct call fails this sends the mask key first. winit sends Alt instead, which Windows can
/// take as a lone key press and answer by opening the Start menu.
pub fn take_foreground(window: isize) -> bool {
    let window = window as HWND;
    let is_foreground = || unsafe { GetForegroundWindow() } == window;
    if is_foreground() {
        return true;
    }
    unsafe { SetForegroundWindow(window) };
    if is_foreground() {
        return true;
    }
    tap_key(MASK_KEY);
    unsafe { SetForegroundWindow(window) };
    is_foreground()
}

/// Gives the foreground back to `window` when it can still take it: it exists, is visible and
/// not minimized, and is not cloaked (the Start menu and other shell windows stay "visible"
/// while hidden). Call it while this process is in the foreground.
pub fn restore_foreground(window: isize) {
    let window = window as HWND;
    if window.is_null()
        || unsafe { IsWindow(window) } == 0
        || unsafe { IsWindowVisible(window) } == 0
        || unsafe { IsIconic(window) } != 0
    {
        return;
    }
    let mut cloaked = 0u32;
    let attribute = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    if attribute != 0 || cloaked == 0 {
        unsafe { SetForegroundWindow(window) };
    }
}

/// Work area and DPI scale of the primary monitor.
pub fn primary_work_area() -> Option<WorkArea> {
    let origin = POINT { x: 0, y: 0 };
    monitor_work_area(unsafe { MonitorFromPoint(origin, MONITOR_DEFAULTTOPRIMARY) })
}

fn monitor_work_area(monitor: HMONITOR) -> Option<WorkArea> {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }

    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };

    let rect = info.rcWork;
    Some(WorkArea {
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left).max(0) as u32,
        height: (rect.bottom - rect.top).max(0) as u32,
        scale: dpi_x as f32 / 96.0,
    })
}

/// Opens a file or folder with its default handler, like a double click in Explorer.
pub fn open_path(path: &str) -> io::Result<()> {
    let operation = wide("open");
    let file = wide(path);
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Opens Explorer on the folder that contains `path`, with the item selected.
pub fn reveal_in_folder(path: &str) -> io::Result<()> {
    Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .map(drop)
}

/// Opens a file for reading and refuses other writers, deletes and renames until it is closed.
pub fn open_locked(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

/// The user's locale name, such as "pt-BR". Empty if Windows does not report one.
pub fn user_language() -> String {
    let mut name = [0u16; LOCALE_NAME_MAX_LENGTH as usize];
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if length <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&name[..length as usize - 1])
}

/// Whether Windows asks apps for the dark theme (Settings > Personalization > Colors).
pub fn system_uses_dark_theme() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut light: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut size,
        )
    };
    status == 0 && light == 0
}

/// Converts a Unix time in milliseconds to the user's local time.
pub fn local_time(unix_ms: i64) -> LocalTime {
    let ticks = unix_ms_to_filetime(unix_ms);
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let converted = unsafe { FileTimeToSystemTime(&file_time, &mut utc) } != 0
        && unsafe { SystemTimeToTzSpecificLocalTime(ptr::null(), &utc, &mut local) } != 0;
    let time = if converted { local } else { utc };
    LocalTime {
        year: time.wYear,
        month: time.wMonth as u8,
        day: time.wDay as u8,
        hour: time.wHour as u8,
        minute: time.wMinute as u8,
    }
}

/// Unix milliseconds as a FILETIME: 100 ns ticks since 1601, clamped to the range Windows accepts.
fn unix_ms_to_filetime(unix_ms: i64) -> u64 {
    const EPOCH_GAP_MS: i64 = 11_644_473_600_000;
    let ms = unix_ms.saturating_add(EPOCH_GAP_MS).max(0) as u64;
    ms.saturating_mul(10_000).min(i64::MAX as u64)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// `%APPDATA%\Tex`: settings, which roam with the user.
pub fn config_dir() -> PathBuf {
    known_folder("APPDATA", "Roaming").join("Tex")
}

/// `%LOCALAPPDATA%\Tex`: the clipboard history, which stays on this machine.
pub fn data_dir() -> PathBuf {
    known_folder("LOCALAPPDATA", "Local").join("Tex")
}

/// `%LOCALAPPDATA%\Tex`: downloaded updates.
pub fn cache_dir() -> PathBuf {
    data_dir()
}

fn known_folder(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                .join("AppData")
                .join(fallback)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_counts_100ns_from_1601() {
        assert_eq!(unix_ms_to_filetime(0), 116_444_736_000_000_000);
        assert_eq!(unix_ms_to_filetime(1), 116_444_736_000_010_000);
        assert_eq!(unix_ms_to_filetime(-11_644_473_600_000), 0);
        assert_eq!(unix_ms_to_filetime(i64::MIN), 0);
    }

    #[test]
    fn local_time_of_the_epoch_is_around_1970() {
        let time = local_time(0);
        assert!(time.year == 1969 || time.year == 1970);
        assert!((1..=12).contains(&time.month));
        assert!((1..=31).contains(&time.day));
        assert!(time.hour < 24);
        assert!(time.minute < 60);
    }

    #[test]
    fn a_locked_file_cannot_be_written_or_deleted() {
        let path = std::env::temp_dir().join(format!("tex-locked-{}", std::process::id()));
        std::fs::write(&path, b"setup").unwrap();
        let file = open_locked(&path).unwrap();
        assert!(std::fs::write(&path, b"evil").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        assert!(std::fs::rename(&path, path.with_extension("moved")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"setup");
        drop(file);
        std::fs::remove_file(&path).unwrap();
    }
}
