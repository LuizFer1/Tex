use std::io;

use super::{keys, x11};
use crate::{Accelerator, HotkeyEvent, HotkeySlot, HotkeySupport};

/// How this session registers global shortcuts, picked once at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    X11,
    Shell,
    Portal,
    Unavailable,
}

/// X11 grabs the keys itself; on Wayland, GNOME goes through the extension and the other
/// desktops through the GlobalShortcuts portal when they have it.
pub(crate) fn choose(x11: bool, gnome: bool, portal: impl FnOnce() -> bool) -> Kind {
    if x11 {
        Kind::X11
    } else if gnome {
        Kind::Shell
    } else if portal() {
        Kind::Portal
    } else {
        Kind::Unavailable
    }
}

enum Backend {
    X11(keys::Grabs),
    Unavailable,
}

/// The two global shortcuts, through whatever this session offers.
pub struct Hotkeys {
    backend: Backend,
}

impl Hotkeys {
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> io::Result<Hotkeys> {
        let pressed = move |slot| on_event(HotkeyEvent::Pressed(slot));
        let backend = match choose(x11::is_x11(), false, || false) {
            Kind::X11 => keys::Grabs::start(pressed).map_or(Backend::Unavailable, Backend::X11),
            Kind::Shell | Kind::Portal | Kind::Unavailable => Backend::Unavailable,
        };
        Ok(Hotkeys { backend })
    }

    pub fn support(&self) -> HotkeySupport {
        match &self.backend {
            Backend::X11(_) => HotkeySupport::Free,
            Backend::Unavailable => HotkeySupport::Unavailable,
        }
    }

    /// Registers `accelerator` for `slot`, replacing what the slot had.
    pub fn register(&mut self, slot: HotkeySlot, accelerator: &Accelerator) -> io::Result<()> {
        match &mut self.backend {
            Backend::X11(grabs) => grabs.grab(slot, accelerator.keysym, accelerator.mods),
            Backend::Unavailable => Err(unavailable()),
        }
    }

    pub fn unregister(&mut self, slot: HotkeySlot) {
        match &mut self.backend {
            Backend::X11(grabs) => grabs.ungrab(slot),
            Backend::Unavailable => {}
        }
    }
}

fn unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "global shortcuts are not available in this session",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_grabs_its_own_keys() {
        assert_eq!(choose(true, true, || true), Kind::X11);
        assert_eq!(choose(true, false, || false), Kind::X11);
    }

    #[test]
    fn gnome_on_wayland_goes_through_the_extension() {
        assert_eq!(choose(false, true, || true), Kind::Shell);
    }

    #[test]
    fn other_wayland_desktops_use_the_portal_when_they_have_it() {
        assert_eq!(choose(false, false, || true), Kind::Portal);
        assert_eq!(choose(false, false, || false), Kind::Unavailable);
    }

    #[test]
    fn the_portal_is_not_asked_when_another_way_exists() {
        let asked = std::cell::Cell::new(false);
        choose(true, false, || {
            asked.set(true);
            true
        });
        choose(false, true, || {
            asked.set(true);
            true
        });
        assert!(!asked.get());
    }
}
