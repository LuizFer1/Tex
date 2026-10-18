use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;

use super::super::dbus::session_bus;
use super::super::shell::{INTERFACE, NAME, PATH};
use super::mime::{self, TEX_MIME};
use super::{OnChange, Wanted, Writer};

/// The copies Tex's GNOME Shell extension announces with its `Offer` signal. The extension
/// may come later: the bus delivers its signals whenever it is there.
pub(crate) struct Offers {
    bus: Connection,
    signals: MessageIterator,
}

pub(crate) fn open() -> io::Result<Offers> {
    let bus = session_bus()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no session bus"))?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(NAME)
        .and_then(|rule| rule.interface(INTERFACE))
        .and_then(|rule| rule.member("Offer"))
        .map_err(io::Error::other)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, &bus, None).map_err(io::Error::other)?;
    Ok(Offers { bus, signals })
}

impl Offers {
    /// Reads each announced copy through `Deliver` until `stopped` is set.
    pub(crate) fn run(self, stopped: Arc<AtomicBool>, wanted: Wanted, on_change: OnChange) {
        for message in self.signals.flatten() {
            if stopped.load(Ordering::SeqCst) {
                return;
            }
            let Ok((serial, mimes, owner)) =
                message.body().deserialize::<(u32, Vec<String>, String)>()
            else {
                continue;
            };
            let owner = (!owner.is_empty()).then_some(owner);
            let bus = &self.bus;
            let clip = mime::read(&mimes, owner, &*wanted, &mut |name| {
                deliver(bus, serial, name)
            });
            if let Some(clip) = clip {
                on_change(clip);
            }
        }
    }
}

/// One target of the copy numbered `serial`; nothing once a newer copy replaced it.
fn deliver(bus: &Connection, serial: u32, mime: &str) -> Option<Vec<u8>> {
    let reply = bus
        .call_method(
            Some(NAME),
            PATH,
            Some(INTERFACE),
            "Deliver",
            &(serial, mime),
        )
        .ok()?;
    reply.body().deserialize::<Vec<u8>>().ok()
}

/// Copies through the extension, which skips its own copies: only the content target goes.
pub(crate) fn writer() -> Writer {
    Box::new(|items| {
        let bus = session_bus()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no session bus"))?;
        let (mime, bytes) = items
            .into_iter()
            .find(|(mime, _)| mime != TEX_MIME)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to copy"))?;
        bus.call_method(
            Some(NAME),
            PATH,
            Some(INTERFACE),
            "SetClipboard",
            &(mime, bytes),
        )
        .map(drop)
        .map_err(io::Error::other)
    })
}
