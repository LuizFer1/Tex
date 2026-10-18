use std::io;
use std::sync::{Arc, Mutex};
use std::thread;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::ReplyError;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask, Window};
use x11rb::protocol::{ErrorKind, Event};
use x11rb::rust_connection::RustConnection;

use crate::HotkeySlot;

/// Shift, Control, Mod1 (Alt) and Mod4 (Super): the modifiers a shortcut can use.
const SHORTCUT_MODS: u16 = 1 | 4 | 8 | 64;
/// Lock (Caps Lock) and Mod2 (Num Lock), which X11 counts as modifiers when on.
const LOCK_MODS: [u16; 4] = [0, 2, 16, 2 | 16];

/// The key a slot grabbed: keycode, modifiers and whether it is held down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Grab {
    keycode: u8,
    mods: u16,
    down: bool,
}

/// Global shortcuts on X11: the keys are grabbed on the root window of a connection of their
/// own, and a thread blocks on its events.
pub(crate) struct Grabs {
    connection: Arc<RustConnection>,
    root: Window,
    grabs: Arc<Mutex<[Option<Grab>; 2]>>,
}

impl Grabs {
    pub(crate) fn start(on_press: impl Fn(HotkeySlot) + Send + 'static) -> Option<Grabs> {
        let (connection, screen) = x11rb::connect(None).ok()?;
        let root = connection.setup().roots.get(screen)?.root;
        if connection
            .extension_information(xkb::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some()
        {
            let _ = connection
                .xkb_use_extension(1, 0)
                .ok()
                .and_then(|cookie| cookie.reply().ok());
            let _ = connection
                .xkb_per_client_flags(
                    xkb::ID::USE_CORE_KBD.into(),
                    xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
                    xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
                    Default::default(),
                    Default::default(),
                    Default::default(),
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok());
        }
        let connection = Arc::new(connection);
        let grabs = Arc::new(Mutex::new([None; 2]));
        thread::spawn({
            let connection = connection.clone();
            let grabs = grabs.clone();
            move || {
                while let Ok(event) = connection.wait_for_event() {
                    let (keycode, state, pressed) = match event {
                        Event::KeyPress(event) => (event.detail, u16::from(event.state), true),
                        Event::KeyRelease(event) => (event.detail, u16::from(event.state), false),
                        _ => continue,
                    };
                    let slot = key_event(&mut grabs.lock().unwrap(), keycode, state, pressed);
                    if let Some(slot) = slot {
                        on_press(slot);
                    }
                }
            }
        });
        Some(Grabs {
            connection,
            root,
            grabs,
        })
    }

    /// Grabs the key with `keysym` and `mods` for `slot`, replacing what the slot had.
    pub(crate) fn grab(&self, slot: HotkeySlot, keysym: u32, mods: u16) -> io::Result<()> {
        self.ungrab(slot);
        let keycode = self
            .keycode(keysym)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no key for this shortcut"))?;
        for lock in LOCK_MODS {
            let checked = self
                .connection
                .grab_key(
                    false,
                    self.root,
                    ModMask::from(mods | lock),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(io::Error::other)?
                .check();
            if let Err(error) = checked {
                self.release(keycode, mods);
                return Err(match error {
                    ReplyError::X11Error(error) if error.error_kind == ErrorKind::Access => {
                        io::Error::new(io::ErrorKind::AlreadyExists, "shortcut already grabbed")
                    }
                    error => io::Error::other(error),
                });
            }
        }
        self.grabs.lock().unwrap()[slot.index()] = Some(Grab {
            keycode,
            mods,
            down: false,
        });
        Ok(())
    }

    pub(crate) fn ungrab(&self, slot: HotkeySlot) {
        let grab = self.grabs.lock().unwrap()[slot.index()].take();
        if let Some(grab) = grab {
            self.release(grab.keycode, grab.mods);
        }
    }

    fn release(&self, keycode: u8, mods: u16) {
        for lock in LOCK_MODS {
            let _ = self
                .connection
                .ungrab_key(keycode, self.root, ModMask::from(mods | lock));
        }
        let _ = self.connection.flush();
    }

    fn keycode(&self, keysym: u32) -> Option<u8> {
        let setup = self.connection.setup();
        let first = setup.min_keycode;
        let count = setup.max_keycode.checked_sub(first)? + 1;
        let mapping = self
            .connection
            .get_keyboard_mapping(first, count)
            .ok()?
            .reply()
            .ok()?;
        keycode_for(keysym, first, mapping.keysyms_per_keycode, &mapping.keysyms)
    }
}

impl Drop for Grabs {
    fn drop(&mut self) {
        self.ungrab(HotkeySlot::Tex);
        self.ungrab(HotkeySlot::History);
    }
}

/// The first keycode whose keysyms include `keysym`, in a keyboard mapping that starts at
/// `first` with `per_keycode` keysyms for each keycode.
pub(crate) fn keycode_for(keysym: u32, first: u8, per_keycode: u8, keysyms: &[u32]) -> Option<u8> {
    if per_keycode == 0 {
        return None;
    }
    let index = keysyms
        .chunks(usize::from(per_keycode))
        .position(|symbols| symbols.contains(&keysym))?;
    u8::try_from(usize::from(first) + index).ok()
}

/// Applies a key event to the grabs and returns the slot to fire: a press of a grabbed key
/// with exactly its modifiers (locks ignored), once until the key is released, so holding it
/// does not repeat.
pub(crate) fn key_event(
    grabs: &mut [Option<Grab>; 2],
    keycode: u8,
    state: u16,
    pressed: bool,
) -> Option<HotkeySlot> {
    let mods = state & SHORTCUT_MODS;
    let mut fired = None;
    for (index, grab) in grabs.iter_mut().enumerate() {
        let Some(grab) = grab.as_mut().filter(|grab| grab.keycode == keycode) else {
            continue;
        };
        if !pressed {
            grab.down = false;
        } else if grab.mods == mods && !grab.down {
            grab.down = true;
            fired = Some(if index == 0 {
                HotkeySlot::Tex
            } else {
                HotkeySlot::History
            });
        }
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_keycode_of_a_keysym() {
        let keysyms = [0x61, 0x41, 0, 0x20, 0, 0, 0x76, 0x56, 0];
        assert_eq!(keycode_for(0x61, 8, 3, &keysyms), Some(8));
        assert_eq!(keycode_for(0x20, 8, 3, &keysyms), Some(9));
        assert_eq!(keycode_for(0x56, 8, 3, &keysyms), Some(10));
        assert_eq!(keycode_for(0xffbe, 8, 3, &keysyms), None);
        assert_eq!(keycode_for(0x61, 8, 0, &keysyms), None);
        assert_eq!(keycode_for(0x76, 254, 3, &keysyms), None);
    }

    fn grabs() -> [Option<Grab>; 2] {
        let grab = |keycode, mods| {
            Some(Grab {
                keycode,
                mods,
                down: false,
            })
        };
        [grab(65, 8), grab(55, 8)]
    }

    #[test]
    fn a_press_with_the_exact_modifiers_fires_its_slot() {
        let mut grabs = grabs();
        assert_eq!(key_event(&mut grabs, 65, 8, true), Some(HotkeySlot::Tex));
        assert_eq!(key_event(&mut grabs, 65, 8, false), None);
        assert_eq!(
            key_event(&mut grabs, 55, 8, true),
            Some(HotkeySlot::History)
        );
        assert_eq!(key_event(&mut grabs, 55, 8 | 1, true), None);
        assert_eq!(key_event(&mut grabs, 66, 8, true), None);
    }

    #[test]
    fn caps_lock_and_num_lock_do_not_matter() {
        let mut grabs = grabs();
        assert_eq!(
            key_event(&mut grabs, 65, 8 | 2 | 16, true),
            Some(HotkeySlot::Tex)
        );
    }

    #[test]
    fn holding_the_key_fires_once_until_released() {
        let mut grabs = grabs();
        assert_eq!(key_event(&mut grabs, 65, 8, true), Some(HotkeySlot::Tex));
        assert_eq!(key_event(&mut grabs, 65, 8, true), None);
        assert_eq!(key_event(&mut grabs, 65, 8, false), None);
        assert_eq!(key_event(&mut grabs, 65, 8, true), Some(HotkeySlot::Tex));
    }
}
