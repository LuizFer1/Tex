use std::io;

use wayland_client::Connection;

use super::wake::Inbox;
use super::{OnChange, Wanted};

/// A data-control manager the compositor offers: `ext-data-control-v1`, else the older
/// `zwlr-data-control-unstable-v1`.
pub(crate) enum Device {
    Ext(ext::Control),
    Wlr(wlr::Control),
}

/// Connects to the Wayland compositor with a connection of its own and binds a data-control
/// manager for the first seat. Fails when the compositor offers neither protocol (GNOME).
pub(crate) fn open() -> io::Result<Device> {
    let connection = Connection::connect_to_env().map_err(io::Error::other)?;
    if let Ok(control) = ext::Control::open(&connection) {
        return Ok(Device::Ext(control));
    }
    wlr::Control::open(&connection).map(Device::Wlr)
}

impl Device {
    pub(crate) fn run(self, inbox: Inbox, wanted: Wanted, on_change: OnChange) {
        match self {
            Device::Ext(control) => control.run(inbox, wanted, on_change),
            Device::Wlr(control) => control.run(inbox, wanted, on_change),
        }
    }
}

macro_rules! data_control {
    ($module:ident, $($protocol:ident)::+, $manager:ident :: $Manager:ident, $device:ident :: $Device:ident, $source:ident :: $Source:ident, $offer:ident :: $Offer:ident) => {
        pub(crate) mod $module {
            use std::collections::HashMap;
            use std::fs::File;
            use std::io::{self, Write};
            use std::os::fd::{AsFd, AsRawFd};
            use std::sync::Arc;
            use std::thread;

            use wayland_client::globals::{GlobalListContents, registry_queue_init};
            use wayland_client::protocol::{wl_registry, wl_seat};
            use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
            use $($protocol)::+::$device::{self, $Device};
            use $($protocol)::+::$manager::$Manager;
            use $($protocol)::+::$offer::{self, $Offer};
            use $($protocol)::+::$source::{self, $Source};

            use super::super::mime;
            use super::super::wake::{Inbox, Message, read_all, wait};
            use super::super::{OnChange, READ_LIMIT, STALL, Wanted};

            #[derive(Default)]
            struct State {
                offers: HashMap<wayland_client::backend::ObjectId, Vec<String>>,
                selection: Option<($Offer, Vec<String>)>,
                source: Option<$Source>,
                items: Arc<Vec<(String, Vec<u8>)>>,
                finished: bool,
            }

            pub(crate) struct Control {
                connection: Connection,
                queue: EventQueue<State>,
                manager: $Manager,
                device: $Device,
                state: State,
            }

            impl Control {
                pub(crate) fn open(connection: &Connection) -> io::Result<Control> {
                    let (globals, mut queue) =
                        registry_queue_init::<State>(connection).map_err(io::Error::other)?;
                    let handle = queue.handle();
                    let manager: $Manager = globals
                        .bind(&handle, 1..=1, ())
                        .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
                    let seat: wl_seat::WlSeat = globals
                        .bind(&handle, 1..=1, ())
                        .map_err(io::Error::other)?;
                    let device = manager.get_data_device(&seat, &handle, ());
                    let mut state = State::default();
                    queue.roundtrip(&mut state).map_err(io::Error::other)?;
                    Ok(Control {
                        connection: connection.clone(),
                        queue,
                        manager,
                        device,
                        state,
                    })
                }

                /// Serves the clipboard until a [`Message::Stop`] arrives or the compositor
                /// takes the device away.
                pub(crate) fn run(mut self, inbox: Inbox, wanted: Wanted, on_change: OnChange) {
                    if let Some((offer, _)) = self.state.selection.take() {
                        offer.destroy();
                    }
                    loop {
                        if self.queue.dispatch_pending(&mut self.state).is_err() || self.state.finished {
                            return;
                        }
                        if let Some((offer, mimes)) = self.state.selection.take() {
                            let connection = &self.connection;
                            let clip = mime::read(&mimes, None, &*wanted, &mut |name| {
                                receive(connection, &offer, name)
                            });
                            offer.destroy();
                            if let Some(clip) = clip {
                                on_change(clip);
                            }
                            continue;
                        }
                        if self.queue.flush().is_err() {
                            return;
                        }
                        let Some(guard) = self.queue.prepare_read() else {
                            continue;
                        };
                        let fd = guard.connection_fd().as_raw_fd();
                        let Ok(ready) = wait(&[fd, inbox.fd()], None) else {
                            return;
                        };
                        if ready[0] {
                            if guard.read().is_err() {
                                return;
                            }
                        } else {
                            drop(guard);
                        }
                        for message in inbox.take() {
                            match message {
                                Message::Write(items) => self.own(items),
                                Message::Stop => {
                                    self.device.destroy();
                                    let _ = self.queue.flush();
                                    return;
                                }
                            }
                        }
                    }
                }

                fn own(&mut self, items: Vec<(String, Vec<u8>)>) {
                    let source = self.manager.create_data_source(&self.queue.handle(), ());
                    for (mime, _) in &items {
                        source.offer(mime.clone());
                    }
                    self.device.set_selection(Some(&source));
                    if let Some(old) = self.state.source.replace(source) {
                        old.destroy();
                    }
                    self.state.items = Arc::new(items);
                }
            }

            /// Reads one target of an offer through a pipe.
            fn receive(connection: &Connection, offer: &$Offer, mime: &str) -> Option<Vec<u8>> {
                let (reader, writer) = io::pipe().ok()?;
                offer.receive(mime.to_owned(), writer.as_fd());
                connection.flush().ok()?;
                drop(writer);
                read_all(reader, READ_LIMIT, STALL)
            }

            impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
                fn event(
                    _: &mut Self,
                    _: &wl_registry::WlRegistry,
                    _: wl_registry::Event,
                    _: &GlobalListContents,
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            impl Dispatch<wl_seat::WlSeat, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &wl_seat::WlSeat,
                    _: wl_seat::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            impl Dispatch<$Manager, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &$Manager,
                    _: <$Manager as Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            impl Dispatch<$Device, ()> for State {
                fn event(
                    state: &mut Self,
                    _: &$Device,
                    event: $device::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    match event {
                        $device::Event::DataOffer { id } => {
                            state.offers.insert(id.id(), Vec::new());
                        }
                        $device::Event::Selection { id } => {
                            if let Some((old, _)) = state.selection.take() {
                                old.destroy();
                            }
                            if let Some(offer) = id {
                                let mimes = state.offers.remove(&offer.id()).unwrap_or_default();
                                state.selection = Some((offer, mimes));
                            }
                        }
                        $device::Event::PrimarySelection { id: Some(offer) } => {
                            state.offers.remove(&offer.id());
                            offer.destroy();
                        }
                        $device::Event::Finished => state.finished = true,
                        _ => {}
                    }
                }

                event_created_child!(State, $Device, [
                    $device::EVT_DATA_OFFER_OPCODE => ($Offer, ()),
                ]);
            }

            impl Dispatch<$Offer, ()> for State {
                fn event(
                    state: &mut Self,
                    offer: &$Offer,
                    event: $offer::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    if let $offer::Event::Offer { mime_type } = event
                        && let Some(mimes) = state.offers.get_mut(&offer.id())
                    {
                        mimes.push(mime_type);
                    }
                }
            }

            impl Dispatch<$Source, ()> for State {
                fn event(
                    state: &mut Self,
                    source: &$Source,
                    event: $source::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    match event {
                        $source::Event::Send { mime_type, fd } => {
                            let items = Arc::clone(&state.items);
                            thread::spawn(move || {
                                if let Some((_, bytes)) = items.iter().find(|(mime, _)| *mime == mime_type) {
                                    let _ = File::from(fd).write_all(bytes);
                                }
                            });
                        }
                        $source::Event::Cancelled => {
                            source.destroy();
                            if state.source.as_ref() == Some(source) {
                                state.source = None;
                                state.items = Arc::default();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    };
}

data_control!(
    ext,
    wayland_protocols::ext::data_control::v1::client,
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_device_v1::ExtDataControlDeviceV1,
    ext_data_control_source_v1::ExtDataControlSourceV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1
);

data_control!(
    wlr,
    wayland_protocols_wlr::data_control::v1::client,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
    zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1
);
