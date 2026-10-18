use std::io;
use std::sync::Arc;

use super::{keys, portal, shell, x11};
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
    /// The GNOME accelerator of each slot, empty when not registered.
    Shell([String; 2]),
    Portal(portal::Shortcuts),
    Unavailable,
}

/// The two global shortcuts, through whatever this session offers.
pub struct Hotkeys {
    backend: Backend,
}

impl Hotkeys {
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> io::Result<Hotkeys> {
        let on_event: Arc<dyn Fn(HotkeyEvent) + Send + Sync> = Arc::new(on_event);
        let pressed = {
            let on_event = on_event.clone();
            move |slot| on_event(HotkeyEvent::Pressed(slot))
        };
        let backend = match choose(x11::is_x11(), shell::is_gnome(), portal::available) {
            Kind::X11 => keys::Grabs::start(pressed).map_or(Backend::Unavailable, Backend::X11),
            Kind::Shell => {
                shell::watch_shortcuts(pressed);
                let on_event = on_event.clone();
                shell::watch(move |_| on_event(HotkeyEvent::Changed));
                Backend::Shell([String::new(), String::new()])
            }
            Kind::Portal => Backend::Portal(portal::Shortcuts::start(on_event)),
            Kind::Unavailable => Backend::Unavailable,
        };
        Ok(Hotkeys { backend })
    }

    pub fn support(&self) -> HotkeySupport {
        match &self.backend {
            Backend::X11(_) => HotkeySupport::Free,
            Backend::Shell(_) if shell::present() => HotkeySupport::Free,
            Backend::Portal(_) => HotkeySupport::System,
            Backend::Shell(_) | Backend::Unavailable => HotkeySupport::Unavailable,
        }
    }

    /// Registers `accelerator` for `slot`, replacing what the slot had. The portal answers
    /// later with [`HotkeyEvent::Bound`], so for it this only asks.
    pub fn register(&mut self, slot: HotkeySlot, accelerator: &Accelerator) -> io::Result<()> {
        match &mut self.backend {
            Backend::X11(grabs) => grabs.grab(slot, accelerator.keysym, accelerator.mods),
            Backend::Shell(wanted) => {
                wanted[slot.index()] = accelerator.gnome.clone();
                send_to_shell(wanted, slot)
            }
            Backend::Portal(shortcuts) => {
                shortcuts.set(
                    slot,
                    Some((accelerator.label.clone(), accelerator.portal.clone())),
                );
                Ok(())
            }
            Backend::Unavailable => Err(unavailable()),
        }
    }

    pub fn unregister(&mut self, slot: HotkeySlot) {
        match &mut self.backend {
            Backend::X11(grabs) => grabs.ungrab(slot),
            Backend::Shell(wanted) => {
                if !std::mem::take(&mut wanted[slot.index()]).is_empty() {
                    let _ = shell::set_shortcuts(&wanted[0], &wanted[1]);
                }
            }
            Backend::Portal(shortcuts) => shortcuts.set(slot, None),
            Backend::Unavailable => {}
        }
    }
}

fn send_to_shell(wanted: &[String; 2], slot: HotkeySlot) -> io::Result<()> {
    let (tex, history) = shell::set_shortcuts(&wanted[0], &wanted[1]).ok_or_else(unavailable)?;
    if [tex, history][slot.index()] {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "shortcut already used by GNOME",
        ))
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
