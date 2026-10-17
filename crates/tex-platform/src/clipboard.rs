use std::cell::RefCell;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CLASS_ALREADY_EXISTS, GetLastError, GlobalFree, HANDLE, HWND, LPARAM,
    LRESULT, WPARAM,
};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    GetClipboardOwner, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    RemoveClipboardFormatListener, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_UNICODETEXT};
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, MSG, PostThreadMessageW, RegisterClassW,
    SetForegroundWindow, TranslateMessage, WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
};

const TEX_FORMAT: &str = "Tex.Copy";
const EXCLUDE_FORMAT: &str = "ExcludeClipboardContentFromMonitorProcessing";
const HISTORY_FORMAT: &str = "CanIncludeInClipboardHistory";
const PNG_FORMAT: &str = "PNG";
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

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

type Callback = Box<dyn Fn(Clip)>;

thread_local! {
    static ON_CHANGE: RefCell<Option<Callback>> = const { RefCell::new(None) };
}

/// Calls `on_change` on its own thread for every clipboard update. Dropping it stops the thread.
pub struct ClipboardWatcher {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardWatcher {
    pub fn start(on_change: impl Fn(Clip) + Send + 'static) -> io::Result<ClipboardWatcher> {
        let (ready, started) = mpsc::channel();
        let thread = thread::spawn(move || {
            ON_CHANGE.with_borrow_mut(|slot| *slot = Some(Box::new(on_change)));
            let window = match create_window() {
                Ok(window) => window,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            if unsafe { AddClipboardFormatListener(window) } == 0 {
                let _ = ready.send(Err(io::Error::last_os_error()));
                unsafe { DestroyWindow(window) };
                return;
            }
            let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
            let mut message: MSG = unsafe { zeroed() };
            while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            unsafe {
                RemoveClipboardFormatListener(window);
                DestroyWindow(window);
            }
        });
        match started.recv() {
            Ok(Ok(thread_id)) => Ok(ClipboardWatcher {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(io::Error::other("clipboard watcher thread stopped")),
        }
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn create_window() -> io::Result<HWND> {
    let class = wide("TexClipboardWatcher");
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let mut info: WNDCLASSW = unsafe { zeroed() };
    info.lpfnWndProc = Some(window_proc);
    info.hInstance = instance;
    info.lpszClassName = class.as_ptr();
    if unsafe { RegisterClassW(&info) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(io::Error::last_os_error());
    }
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(window)
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        if let Some(clip) = read_clip(window) {
            ON_CHANGE.with_borrow(|callback| {
                if let Some(callback) = callback {
                    callback(clip);
                }
            });
        }
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Reads the formats Tex cares about. `None` when the clipboard stays locked by another app.
fn read_clip(window: HWND) -> Option<Clip> {
    if !open_clipboard(window) {
        return None;
    }
    let from_tex = available(format(TEX_FORMAT));
    let excluded = available(format(EXCLUDE_FORMAT));
    let history_disallowed = read_format(format(HISTORY_FORMAT))
        .and_then(|bytes| bytes.get(..4).map(|value| value == [0, 0, 0, 0]))
        .unwrap_or(false);
    let (text, dib) = if from_tex {
        (None, None)
    } else {
        let dib = read_format(CF_DIBV5 as u32).or_else(|| read_format(CF_DIB as u32));
        (read_text(), dib)
    };
    unsafe { CloseClipboard() };
    Some(Clip {
        text,
        dib,
        excluded,
        history_disallowed,
        from_tex,
        owner_exe: owner_exe(),
    })
}

fn open_clipboard(window: HWND) -> bool {
    for attempt in 0..OPEN_ATTEMPTS {
        if unsafe { OpenClipboard(window) } != 0 {
            return true;
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            thread::sleep(OPEN_RETRY);
        }
    }
    false
}

fn format(name: &str) -> u32 {
    let name = wide(name);
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn available(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format) } != 0
}

fn read_format(format: u32) -> Option<Vec<u8>> {
    if !available(format) {
        return None;
    }
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    let data = unsafe { GlobalLock(handle) } as *const u8;
    if data.is_null() {
        return None;
    }
    let size = unsafe { GlobalSize(handle) };
    let bytes = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    Some(bytes)
}

fn read_text() -> Option<String> {
    if !available(CF_UNICODETEXT as u32) {
        return None;
    }
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT as u32) };
    if handle.is_null() {
        return None;
    }
    let data = unsafe { GlobalLock(handle) } as *const u16;
    if data.is_null() {
        return None;
    }
    let units = unsafe { GlobalSize(handle) } / 2;
    let slice = unsafe { std::slice::from_raw_parts(data, units) };
    let length = slice.iter().position(|&c| c == 0).unwrap_or(units);
    let text = String::from_utf16_lossy(&slice[..length]);
    unsafe { GlobalUnlock(handle) };
    Some(text)
}

fn owner_exe() -> Option<String> {
    let mut window = unsafe { GetClipboardOwner() };
    if window.is_null() {
        window = unsafe { GetForegroundWindow() };
    }
    if window.is_null() {
        return None;
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    if process_id == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let mut path = [0u16; 1024];
    let mut length = path.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
    } != 0;
    unsafe { CloseHandle(process) };
    if !ok {
        return None;
    }
    let path = String::from_utf16_lossy(&path[..length as usize]);
    let name = path.rsplit('\\').next().unwrap_or(&path);
    (!name.is_empty()).then(|| name.to_string())
}

/// Replaces the clipboard with `text`, marked as a Tex copy.
pub fn write_text(text: &str) -> io::Result<()> {
    let bytes: Vec<u8> = text
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    write(&[(CF_UNICODETEXT as u32, &bytes), (format(TEX_FORMAT), &[1])])
}

/// Replaces the clipboard with an image as `CF_DIB` and `PNG`, marked as a Tex copy.
pub fn write_image(dib: &[u8], png: &[u8]) -> io::Result<()> {
    write(&[
        (CF_DIB as u32, dib),
        (format(PNG_FORMAT), png),
        (format(TEX_FORMAT), &[1]),
    ])
}

fn write(items: &[(u32, &[u8])]) -> io::Result<()> {
    if !open_clipboard(ptr::null_mut()) {
        return Err(io::Error::last_os_error());
    }
    let result = fill(items);
    unsafe { CloseClipboard() };
    result
}

fn fill(items: &[(u32, &[u8])]) -> io::Result<()> {
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error());
    }
    for &(format, bytes) in items {
        if format == 0 {
            return Err(io::Error::last_os_error());
        }
        let memory = global_copy(bytes)?;
        if unsafe { SetClipboardData(format, memory) }.is_null() {
            let error = io::Error::last_os_error();
            unsafe { GlobalFree(memory) };
            return Err(error);
        }
    }
    Ok(())
}

fn global_copy(bytes: &[u8]) -> io::Result<HANDLE> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) };
    if memory.is_null() {
        return Err(io::Error::last_os_error());
    }
    let data = unsafe { GlobalLock(memory) } as *mut u8;
    if data.is_null() {
        let error = io::Error::last_os_error();
        unsafe { GlobalFree(memory) };
        return Err(error);
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        GlobalUnlock(memory);
    }
    Ok(memory)
}

/// The window in the foreground, to give the focus back to it later.
pub fn foreground_window() -> isize {
    (unsafe { GetForegroundWindow() }) as isize
}

/// Gives `window` the focus back and sends Ctrl+V.
pub fn paste_into(window: isize) {
    if window != 0 {
        unsafe { SetForegroundWindow(window as HWND) };
    }
    let inputs = [
        key_input(VK_CONTROL, 0),
        key_input(VK_V, 0),
        key_input(VK_V, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
}

fn key_input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
