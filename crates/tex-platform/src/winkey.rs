use std::cell::RefCell;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput,
    VK_LWIN, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CallWindowProcW, DispatchMessageW, GWLP_WNDPROC, GetMessageW, HC_ACTION,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PostThreadMessageW, SetWindowLongPtrW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WNDPROC,
};

use crate::WindowRef;

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());
type Callback = Box<dyn Fn() + Send>;
static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);

/// Calls `on_press` when the Windows key is pressed and released on its own, instead of opening
/// the Start menu. Win+other keys keep working. One hook per process; dropping it removes the hook.
///
/// The hook keeps the Windows key from Windows until another key shows it is a combination, and
/// then sends the key-down before that key. Windows does not call the hook while a window of this
/// process has the focus; [`catch_windows_key`] covers that window.
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
        *TRACKER.lock().unwrap_or_else(|e| e.into_inner()) = Tracker::new();
    }
}

struct CaughtWindow {
    window: isize,
    previous: WNDPROC,
    tracker: Tracker,
    on_press: std::rc::Rc<dyn Fn() -> bool>,
}

thread_local! {
    static CAUGHT: RefCell<Vec<CaughtWindow>> = const { RefCell::new(Vec::new()) };
}

/// The low-level hook is not called while a window of this process has the focus, so `window`
/// gets the Windows key itself. Windows decides to open the Start menu on the release of a lone
/// Windows key before the window sees it, so this sends the mask key on the key-down, which
/// makes the press a combination to Windows, and calls `on_press` on the lone release, swallowing
/// it when `on_press` returns true. Call it on the window's own thread, once per window.
pub fn catch_windows_key(window: WindowRef, on_press: impl Fn() -> bool + 'static) {
    let window = window.0;
    if CAUGHT.with_borrow(|caught| caught.iter().any(|entry| entry.window == window)) {
        return;
    }
    let previous = unsafe {
        SetWindowLongPtrW(
            window as HWND,
            GWLP_WNDPROC,
            caught_window_proc as *const () as isize,
        )
    };
    if previous == 0 {
        return;
    }
    CAUGHT.with_borrow_mut(|caught| {
        caught.push(CaughtWindow {
            window,
            previous: unsafe { std::mem::transmute::<isize, WNDPROC>(previous) },
            tracker: Tracker::new(),
            on_press: std::rc::Rc::new(on_press),
        });
    });
}

unsafe extern "system" fn caught_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
    let key = wparam as u16;
    let found = CAUGHT.with_borrow_mut(|caught| {
        let entry = caught
            .iter_mut()
            .find(|entry| entry.window == window as isize)?;
        let action = if (down || up) && key != MASK_KEY {
            entry.tracker.key(key, down)
        } else {
            Action::Pass
        };
        Some((entry.previous, action, std::rc::Rc::clone(&entry.on_press)))
    });
    let Some((previous, action, on_press)) = found else {
        return 0;
    };
    if action == Action::Swallow && lparam & (1 << 30) == 0 {
        tap_key(MASK_KEY);
    }
    if action == Action::Pressed && on_press() {
        return 0;
    }
    unsafe { CallWindowProcW(previous, window, message, wparam, lparam) }
}

fn clear_callback() {
    *CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let message = wparam as u32;
        let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let action = if event.flags & LLKHF_INJECTED != 0 {
            Action::Pass
        } else {
            TRACKER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .key(event.vkCode as u16, down)
        };
        match action {
            Action::Pass => {}
            Action::Swallow => return 1,
            Action::SendWinFirst(win) => send_key_down(win),
            Action::Pressed => {
                if let Some(callback) = CALLBACK.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                {
                    callback();
                }
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Pass,
    Swallow,
    /// Another key while the Windows key is held: send the Windows key-down before it.
    SendWinFirst(u16),
    /// The Windows key was released on its own: swallow it and call back.
    Pressed,
}

/// The Windows key held back from Windows, and whether it was handed over for a combination.
#[derive(Debug)]
struct Tracker {
    held: Option<u16>,
    sent: bool,
}

impl Tracker {
    const fn new() -> Tracker {
        Tracker {
            held: None,
            sent: false,
        }
    }

    fn key(&mut self, key: u16, down: bool) -> Action {
        let win = key == VK_LWIN || key == VK_RWIN;
        match (win, down, self.held) {
            (true, true, None) => {
                self.held = Some(key);
                self.sent = false;
                Action::Swallow
            }
            (true, true, Some(_)) if !self.sent => Action::Swallow,
            (true, false, Some(held)) if held == key => {
                self.held = None;
                if self.sent {
                    Action::Pass
                } else {
                    Action::Pressed
                }
            }
            (false, true, Some(held)) if !self.sent => {
                self.sent = true;
                Action::SendWinFirst(held)
            }
            _ => Action::Pass,
        }
    }
}

/// Sent from inside the hook, the key-down is processed before the key being hooked.
fn send_key_down(key: u16) {
    let input = key_input(key, KEYEVENTF_EXTENDEDKEY);
    unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
}

/// A key Windows assigns nothing to, so pressing it has no effect anywhere, while it still
/// counts as a key: pressed while the Windows key is down, it keeps the Start menu from opening
/// on the release, and sent by this process, it lets the process take the foreground.
pub(crate) const MASK_KEY: u16 = 0xE8;

/// Both events are injected, so the hook passes them on.
pub(crate) fn tap_key(key: u16) {
    let inputs = [key_input(key, 0), key_input(key, KEYEVENTF_KEYUP)];
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

#[cfg(test)]
mod tests {
    use super::*;

    const E: u16 = 0x45;
    const SHIFT: u16 = 0xA0;

    #[test]
    fn a_lone_press_never_reaches_windows() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.key(VK_LWIN, true), Action::Swallow);
        assert_eq!(tracker.key(VK_LWIN, true), Action::Swallow);
        assert_eq!(tracker.key(VK_LWIN, false), Action::Pressed);
        assert_eq!(tracker.key(E, true), Action::Pass);
    }

    #[test]
    fn a_combination_gets_the_windows_key_first() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.key(VK_RWIN, true), Action::Swallow);
        assert_eq!(tracker.key(SHIFT, true), Action::SendWinFirst(VK_RWIN));
        assert_eq!(tracker.key(E, true), Action::Pass);
        assert_eq!(tracker.key(VK_RWIN, true), Action::Pass);
        assert_eq!(tracker.key(E, false), Action::Pass);
        assert_eq!(tracker.key(VK_RWIN, false), Action::Pass);
        assert_eq!(tracker.key(VK_RWIN, true), Action::Swallow);
        assert_eq!(tracker.key(VK_RWIN, false), Action::Pressed);
    }

    #[test]
    fn key_ups_it_did_not_hold_go_through() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.key(VK_LWIN, false), Action::Pass);
        assert_eq!(tracker.key(E, false), Action::Pass);
        assert_eq!(tracker.key(VK_LWIN, true), Action::Swallow);
        assert_eq!(tracker.key(VK_RWIN, true), Action::Swallow);
        assert_eq!(tracker.key(VK_RWIN, false), Action::Pass);
        assert_eq!(tracker.key(VK_LWIN, false), Action::Pressed);
    }
}
