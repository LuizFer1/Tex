use std::mem::{size_of, zeroed};
use std::ptr;
use std::thread;

use windows_sys::Win32::System::Ole::OleInitialize;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const PATH_CAPACITY: usize = 4096;

/// Shows an Open dialog for `.exe` files on its own thread, owned by the window in the
/// foreground, and calls `on_done` there with the chosen path (`None` when cancelled).
pub fn pick_file(
    title: &str,
    filter_name: &str,
    on_done: impl FnOnce(Option<String>) + Send + 'static,
) {
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let filter: Vec<u16> = filter_name
        .encode_utf16()
        .chain(Some(0))
        .chain("*.exe".encode_utf16())
        .chain([0, 0])
        .collect();
    let owner = unsafe { GetForegroundWindow() } as usize;
    thread::spawn(move || {
        unsafe { OleInitialize(ptr::null()) };
        let mut path = vec![0u16; PATH_CAPACITY];
        let mut info: OPENFILENAMEW = unsafe { zeroed() };
        info.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        info.hwndOwner = owner as _;
        info.lpstrFilter = filter.as_ptr();
        info.nFilterIndex = 1;
        info.lpstrFile = path.as_mut_ptr();
        info.nMaxFile = path.len() as u32;
        info.lpstrTitle = title.as_ptr();
        info.Flags = OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
        if unsafe { GetOpenFileNameW(&mut info) } == 0 {
            on_done(None);
            return;
        }
        let length = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        on_done(Some(String::from_utf16_lossy(&path[..length])));
    });
}
