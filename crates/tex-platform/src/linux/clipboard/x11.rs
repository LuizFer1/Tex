use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::time::Instant;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as _};
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
    PropMode, Property, PropertyNotifyEvent, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use super::mime;
use super::wake::{Inbox, Message, wait};
use super::{OnChange, READ_LIMIT, STALL, Wanted};
use crate::{Clip, ClipFormats, Payload};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        INCR,
        _NET_WM_PID,
        TEX_SELECTION,
    }
}

/// Largest piece sent in one `ChangeProperty` of an incremental transfer.
const CHUNK: usize = 1 << 20;

/// An incremental (`INCR`) transfer of Tex's selection to another client.
struct Transfer {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Vec<u8>,
    sent: usize,
}

/// The X11 `CLIPBOARD`, watched through XFixes from a hidden window of its own connection,
/// which also owns the selection when Tex copies.
pub(crate) struct Selection {
    connection: RustConnection,
    window: Window,
    atoms: Atoms,
    names: HashMap<Atom, String>,
    backlog: VecDeque<Event>,
    owned: Vec<(Atom, Vec<u8>)>,
    transfers: Vec<Transfer>,
}

/// Connects to the X server and asks for owner changes of `CLIPBOARD`.
pub(crate) fn open() -> io::Result<Selection> {
    let (connection, screen) = x11rb::connect(None).map_err(io::Error::other)?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| io::Error::other("no X screen"))?
        .root;
    let atoms = Atoms::new(&connection)
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    connection
        .xfixes_query_version(5, 0)
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    if let Ok(cookie) = connection.res_query_version(1, 2) {
        let _ = cookie.reply();
    }
    let window = connection.generate_id().map_err(io::Error::other)?;
    connection
        .create_window(
            0,
            window,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(io::Error::other)?;
    connection
        .xfixes_select_selection_input(
            window,
            atoms.CLIPBOARD,
            SelectionEventMask::SET_SELECTION_OWNER,
        )
        .map_err(io::Error::other)?;
    connection.flush().map_err(io::Error::other)?;
    Ok(Selection {
        connection,
        window,
        atoms,
        names: HashMap::new(),
        backlog: VecDeque::new(),
        owned: Vec::new(),
        transfers: Vec::new(),
    })
}

impl Selection {
    /// Serves the clipboard until a [`Message::Stop`] arrives.
    pub(crate) fn run(mut self, inbox: Inbox, wanted: Wanted, on_change: OnChange) {
        loop {
            let mut events: Vec<Event> = self.backlog.drain(..).collect();
            loop {
                match self.connection.poll_for_event() {
                    Ok(Some(event)) => events.push(event),
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
            if events.is_empty() {
                let fd = self.connection.stream().as_raw_fd();
                if wait(&[fd, inbox.fd()], None).is_err() {
                    return;
                }
            }
            for message in inbox.take() {
                match message {
                    Message::Write(items) => self.own(items),
                    Message::Stop => {
                        let _ = self.connection.destroy_window(self.window);
                        let _ = self.connection.flush();
                        return;
                    }
                }
            }
            let newest = events
                .iter()
                .rposition(|event| matches!(event, Event::XfixesSelectionNotify(_)));
            for (index, event) in events.into_iter().enumerate() {
                match event {
                    Event::XfixesSelectionNotify(notify) => {
                        if Some(index) == newest
                            && notify.selection == self.atoms.CLIPBOARD
                            && notify.owner != self.window
                            && notify.owner != NONE
                            && let Some(clip) =
                                self.read(notify.owner, notify.selection_timestamp, &*wanted)
                        {
                            on_change(clip);
                        }
                    }
                    Event::SelectionRequest(request) => self.serve(request),
                    Event::SelectionClear(clear) if clear.selection == self.atoms.CLIPBOARD => {
                        self.owned.clear();
                        self.transfers.clear();
                    }
                    Event::PropertyNotify(notify) => self.advance(notify),
                    _ => {}
                }
            }
            let _ = self.connection.flush();
        }
    }

    fn read(
        &mut self,
        owner: Window,
        time: Timestamp,
        wanted: &dyn Fn(&ClipFormats) -> Option<Payload>,
    ) -> Option<Clip> {
        let targets = self.convert(self.atoms.TARGETS, time)?;
        let mimes = self.target_names(&targets);
        let owner_exe = self.owner_exe(owner);
        mime::read(&mimes, owner_exe, wanted, &mut |name| {
            let target = self.atom(name)?;
            self.convert(target, time)
        })
    }

    /// The names of the atoms in a `TARGETS` reply.
    fn target_names(&mut self, targets: &[u8]) -> Vec<String> {
        let atoms: Vec<Atom> = targets
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_ne_bytes(*bytes))
            .collect();
        let cookies: Vec<_> = atoms
            .iter()
            .filter(|atom| !self.names.contains_key(*atom))
            .filter_map(|&atom| Some((atom, self.connection.get_atom_name(atom).ok()?)))
            .collect();
        for (atom, cookie) in cookies {
            if let Ok(reply) = cookie.reply() {
                self.names
                    .insert(atom, String::from_utf8_lossy(&reply.name).into_owned());
            }
        }
        atoms
            .iter()
            .filter_map(|atom| self.names.get(atom).cloned())
            .collect()
    }

    fn atom(&mut self, name: &str) -> Option<Atom> {
        if let Some((&atom, _)) = self.names.iter().find(|(_, known)| *known == name) {
            return Some(atom);
        }
        let atom = self
            .connection
            .intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        self.names.insert(atom, name.to_owned());
        Some(atom)
    }

    /// Asks the owner for `target` and reads the answer, incrementally when it is large.
    fn convert(&mut self, target: Atom, time: Timestamp) -> Option<Vec<u8>> {
        let window = self.window;
        let property = self.atoms.TEX_SELECTION;
        let clipboard = self.atoms.CLIPBOARD;
        self.connection.delete_property(window, property).ok()?;
        self.connection
            .convert_selection(window, clipboard, target, property, time)
            .ok()?;
        self.connection.flush().ok()?;
        let notify = self.wait_for(|event| {
            matches!(event, Event::SelectionNotify(notify)
                if notify.requestor == window && notify.selection == clipboard && notify.target == target)
        })?;
        let Event::SelectionNotify(notify) = notify else {
            return None;
        };
        if notify.property == NONE {
            return None;
        }
        let reply = self
            .connection
            .get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
            .ok()?
            .reply()
            .ok()?;
        self.connection.flush().ok()?;
        if reply.type_ != self.atoms.INCR {
            return Some(reply.value);
        }
        let mut data = Vec::new();
        loop {
            self.wait_for(|event| {
                matches!(event, Event::PropertyNotify(notify)
                    if notify.window == window && notify.atom == property && notify.state == Property::NEW_VALUE)
            })?;
            let piece = self
                .connection
                .get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
                .ok()?
                .reply()
                .ok()?;
            self.connection.flush().ok()?;
            if piece.value.is_empty() {
                return Some(data);
            }
            data.extend_from_slice(&piece.value);
            if data.len() > READ_LIMIT {
                return None;
            }
        }
    }

    /// The next event that matches, keeping the others for the main loop. `None` after
    /// [`STALL`] without it.
    fn wait_for(&mut self, matches: impl Fn(&Event) -> bool) -> Option<Event> {
        let deadline = Instant::now() + STALL;
        loop {
            while let Some(event) = self.connection.poll_for_event().ok()? {
                if matches(&event) {
                    return Some(event);
                }
                self.backlog.push_back(event);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let fd = self.connection.stream().as_raw_fd();
            if !wait(&[fd], Some(left)).ok()?[0] {
                return None;
            }
        }
    }

    /// The executable of the window that owns the selection: `_NET_WM_PID`, else the X-Resource
    /// extension's client PID.
    fn owner_exe(&self, owner: Window) -> Option<String> {
        let pid = self
            .connection
            .get_property(
                false,
                owner,
                self.atoms._NET_WM_PID,
                AtomEnum::CARDINAL,
                0,
                1,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
            .or_else(|| {
                let spec = ClientIdSpec {
                    client: owner,
                    mask: ClientIdMask::LOCAL_CLIENT_PID,
                };
                let reply = self
                    .connection
                    .res_query_client_ids(&[spec])
                    .ok()?
                    .reply()
                    .ok()?;
                reply.ids.first()?.value.first().copied()
            })?;
        let link = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        mime::exe_name(&link)
    }

    /// Takes the selection with these targets.
    fn own(&mut self, items: Vec<(String, Vec<u8>)>) {
        let owned: Vec<(Atom, Vec<u8>)> = items
            .into_iter()
            .filter_map(|(name, bytes)| Some((self.atom(&name)?, bytes)))
            .collect();
        self.owned = owned;
        self.transfers.clear();
        let _ =
            self.connection
                .set_selection_owner(self.window, self.atoms.CLIPBOARD, CURRENT_TIME);
        let _ = self.connection.flush();
    }

    /// Answers another client that asks for Tex's selection.
    fn serve(&mut self, request: SelectionRequestEvent) {
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let answered = request.selection == self.atoms.CLIPBOARD
            && !self.owned.is_empty()
            && self.answer(&request, property);
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if answered { property } else { NONE },
        };
        let _ = self
            .connection
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify);
        let _ = self.connection.flush();
    }

    fn answer(&mut self, request: &SelectionRequestEvent, property: Atom) -> bool {
        if request.target == self.atoms.TARGETS {
            let mut targets = vec![self.atoms.TARGETS];
            targets.extend(self.owned.iter().map(|(atom, _)| *atom));
            return self
                .connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .is_ok();
        }
        let Some((_, data)) = self.owned.iter().find(|(atom, _)| *atom == request.target) else {
            return false;
        };
        let limit = self.connection.maximum_request_bytes().saturating_sub(1024);
        if data.len() <= limit {
            return self
                .connection
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    request.target,
                    data,
                )
                .is_ok();
        }
        let data = data.clone();
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let watch = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        if self
            .connection
            .change_window_attributes(request.requestor, &watch)
            .is_err()
            || self
                .connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    self.atoms.INCR,
                    &[size],
                )
                .is_err()
        {
            return false;
        }
        self.transfers.push(Transfer {
            requestor: request.requestor,
            property,
            target: request.target,
            data,
            sent: 0,
        });
        true
    }

    /// Sends the next piece of an incremental transfer once the requestor deleted the last one.
    fn advance(&mut self, notify: PropertyNotifyEvent) {
        if notify.state != Property::DELETE {
            return;
        }
        let Some(index) = self.transfers.iter().position(|transfer| {
            transfer.requestor == notify.window && transfer.property == notify.atom
        }) else {
            return;
        };
        let transfer = &mut self.transfers[index];
        let end = (transfer.sent + CHUNK).min(transfer.data.len());
        let piece = &transfer.data[transfer.sent..end];
        let _ = self.connection.change_property8(
            PropMode::REPLACE,
            transfer.requestor,
            transfer.property,
            transfer.target,
            piece,
        );
        let done = piece.is_empty();
        transfer.sent = end;
        if done {
            self.transfers.remove(index);
        }
        let _ = self.connection.flush();
    }
}
