use std::io;
use std::sync::{Arc, Mutex};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::{Accelerator, HotkeyEvent, HotkeySlot, HotkeySupport};

/// The two global shortcuts, registered with `global-hotkey` on the UI thread, whose message
/// loop delivers the presses.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Arc<Mutex<[Option<HotKey>; 2]>>,
}

impl Hotkeys {
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> io::Result<Hotkeys> {
        let manager = GlobalHotKeyManager::new().map_err(io::Error::other)?;
        let registered: Arc<Mutex<[Option<HotKey>; 2]>> = Arc::new(Mutex::new([None; 2]));
        let ids = registered.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let slot = ids
                .lock()
                .unwrap()
                .iter()
                .position(|hotkey| hotkey.is_some_and(|hotkey| hotkey.id() == event.id));
            match slot {
                Some(0) => on_event(HotkeyEvent::Pressed(HotkeySlot::Tex)),
                Some(_) => on_event(HotkeyEvent::Pressed(HotkeySlot::History)),
                None => {}
            }
        }));
        Ok(Hotkeys {
            manager,
            registered,
        })
    }

    pub fn support(&self) -> HotkeySupport {
        HotkeySupport::Free
    }

    /// Registers `accelerator` for `slot`, replacing what the slot had.
    pub fn register(&mut self, slot: HotkeySlot, accelerator: &Accelerator) -> io::Result<()> {
        self.unregister(slot);
        self.manager
            .register(accelerator.hotkey)
            .map_err(io::Error::other)?;
        self.registered.lock().unwrap()[slot.index()] = Some(accelerator.hotkey);
        Ok(())
    }

    pub fn unregister(&mut self, slot: HotkeySlot) {
        let hotkey = self.registered.lock().unwrap()[slot.index()].take();
        if let Some(hotkey) = hotkey {
            let _ = self.manager.unregister(hotkey);
        }
    }
}
