use std::io;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, POINT,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
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
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY) };

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

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
