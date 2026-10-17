use std::io;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, POINT, SYSTEMTIME,
};
use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::System::SystemServices::LOCALE_NAME_MAX_LENGTH;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SW_SHOWNORMAL};

use crate::WorkArea;

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

/// A wall-clock time in the user's time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
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
}
