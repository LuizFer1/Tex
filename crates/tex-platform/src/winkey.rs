use std::io;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

const MASK_KEY: u16 = 0xE8;

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static SOLO: AtomicBool = AtomicBool::new(false);
type Callback = Box<dyn Fn() + Send>;
static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);

/// Calls `on_press` when the Windows key is pressed and released on its own, instead of opening
/// the Start menu. Win+other keys keep working. One hook per process; dropping it removes the hook.
pub struct WinKeyHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl WinKeyHook {
    pub fn start(on_press: impl Fn() + Send + 'static) -> io::Result<WinKeyHook> {
        *CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(on_press));
        let (ready, started) = mpsc::channel();
        let thread = thread::spawn(move || {
            let hook = unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(ptr::null()), 0)
            };
            if hook.is_null() {
                let _ = ready.send(Err(io::Error::last_os_error()));
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
            unsafe { UnhookWindowsHookEx(hook) };
        });
        match started.recv() {
            Ok(Ok(thread_id)) => Ok(WinKeyHook {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                clear_callback();
                Err(error)
            }
            Err(_) => {
                clear_callback();
                Err(io::Error::other("keyboard hook thread stopped"))
            }
        }
    }
}

impl Drop for WinKeyHook {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        clear_callback();
        WIN_DOWN.store(false, Ordering::Relaxed);
        SOLO.store(false, Ordering::Relaxed);
    }
}

fn clear_callback() {
    *CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if event.flags & LLKHF_INJECTED == 0 && handle(event.vkCode as u16, wparam as u32) {
            return 1;
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Tracks a lone Windows key press. Returns true when the key-up was swallowed and replaced.
fn handle(key: u16, message: u32) -> bool {
    let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
    if key != VK_LWIN && key != VK_RWIN {
        if down && WIN_DOWN.load(Ordering::Relaxed) {
            SOLO.store(false, Ordering::Relaxed);
        }
        return false;
    }
    if down {
        if !WIN_DOWN.swap(true, Ordering::Relaxed) {
            SOLO.store(true, Ordering::Relaxed);
        }
        return false;
    }
    WIN_DOWN.store(false, Ordering::Relaxed);
    if !SOLO.swap(false, Ordering::Relaxed) {
        return false;
    }
    let inputs = [
        key_input(MASK_KEY, 0),
        key_input(MASK_KEY, KEYEVENTF_KEYUP),
        key_input(key, KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if let Some(callback) = CALLBACK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        callback();
    }
    true
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
