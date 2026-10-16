use std::ptr;
use std::thread;

use windows_sys::Win32::Foundation::MAX_PATH;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Ole::OleInitialize;
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// Shows the folder picker on its own thread, owned by the window in the foreground, and
/// calls `on_done` there with the chosen path (`None` when cancelled).
pub fn pick_folder(title: &str, on_done: impl FnOnce(Option<String>) + Send + 'static) {
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let owner = unsafe { GetForegroundWindow() } as usize;
    thread::spawn(move || {
        unsafe { OleInitialize(ptr::null()) };
        let mut name = [0u16; MAX_PATH as usize];
        let info = BROWSEINFOW {
            hwndOwner: owner as _,
            pidlRoot: ptr::null_mut(),
            pszDisplayName: name.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: None,
            lParam: 0,
            iImage: 0,
        };
        let list = unsafe { SHBrowseForFolderW(&info) };
        if list.is_null() {
            on_done(None);
            return;
        }
        let mut path = [0u16; MAX_PATH as usize];
        let found = unsafe { SHGetPathFromIDListW(list, path.as_mut_ptr()) } != 0;
        unsafe { CoTaskMemFree(list as _) };
        let length = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        on_done(found.then(|| String::from_utf16_lossy(&path[..length])));
    });
}
