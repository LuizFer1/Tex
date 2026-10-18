use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::dbus::{DESKTOP, DESKTOP_PATH, session_bus};
use crate::{HotkeyEvent, HotkeySlot};

const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const IDS: [&str; 2] = ["tex", "history"];
/// Changes that arrive this close together (both shortcuts at once) become one bind.
const SETTLE: Duration = Duration::from_millis(150);

/// The label and preferred trigger each slot asks for; `None` for a slot not wanted.
pub(crate) type Wanted = [Option<(String, String)>; 2];

type OnEvent = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

/// Whether the session has the GlobalShortcuts portal.
pub(crate) fn available() -> bool {
    session_bus()
        .and_then(|bus| Proxy::new(&bus, DESKTOP, DESKTOP_PATH, INTERFACE).ok())
        .is_some_and(|portal| portal.get_property::<u32>("version").is_ok())
}

/// Global shortcuts through the portal: one session holds both, and every change closes it
/// and binds what is wanted in a new one, which may make the desktop ask the user.
pub(crate) struct Shortcuts {
    wanted: Arc<Mutex<Wanted>>,
    changed: Sender<()>,
}

impl Shortcuts {
    pub(crate) fn start(on_event: OnEvent) -> Shortcuts {
        let wanted = Arc::new(Mutex::new([None, None]));
        let (changed, changes) = mpsc::channel();
        let session = Arc::new(Mutex::new(None));
        if let Some(bus) = session_bus() {
            thread::spawn({
                let bus = bus.clone();
                let wanted = wanted.clone();
                let session = session.clone();
                let on_event = on_event.clone();
                move || bind_changes(&bus, &changes, &wanted, &session, &*on_event)
            });
            thread::spawn(move || follow_activations(&bus, &session, &*on_event));
        }
        Shortcuts { wanted, changed }
    }

    pub(crate) fn set(&self, slot: HotkeySlot, wanted: Option<(String, String)>) {
        self.wanted.lock().unwrap()[slot.index()] = wanted;
        let _ = self.changed.send(());
    }
}

fn bind_changes(
    bus: &Connection,
    changes: &Receiver<()>,
    wanted: &Mutex<Wanted>,
    session: &Mutex<Option<OwnedObjectPath>>,
    on_event: &(dyn Fn(HotkeyEvent) + Send + Sync),
) {
    let mut sessions = 0u32;
    while changes.recv().is_ok() {
        loop {
            match changes.recv_timeout(SETTLE) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let old = session.lock().unwrap().take();
        if let Some(old) = old {
            close(bus, &old);
        }
        let wanted = wanted.lock().unwrap().clone();
        if wanted.iter().all(Option::is_none) {
            continue;
        }
        sessions += 1;
        let Some(handle) = create_session(bus, &format!("tex{sessions}")) else {
            on_event(HotkeyEvent::Bound {
                tex: None,
                history: None,
            });
            continue;
        };
        *session.lock().unwrap() = Some(handle.clone());
        let [tex, history] = bind(bus, &handle, &wanted).unwrap_or_default();
        on_event(HotkeyEvent::Bound { tex, history });
    }
}

fn create_session(bus: &Connection, token: &str) -> Option<OwnedObjectPath> {
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("session_handle_token", Value::from(token));
    let results = request(bus, |portal| portal.call("CreateSession", &(options,)))?;
    session_handle(&results)
}

fn bind(
    bus: &Connection,
    session: &OwnedObjectPath,
    wanted: &Wanted,
) -> Option<[Option<String>; 2]> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let mut results = request(bus, |portal| {
        portal.call(
            "BindShortcuts",
            &(session, shortcut_list(wanted), "", options),
        )
    })?;
    let bound =
        Vec::<(String, HashMap<String, OwnedValue>)>::try_from(results.remove("shortcuts")?)
            .ok()?;
    Some(triggers(&bound))
}

fn close(bus: &Connection, session: &OwnedObjectPath) {
    if let Ok(proxy) = Proxy::new(
        bus,
        DESKTOP,
        session.as_str(),
        "org.freedesktop.portal.Session",
    ) {
        let _ = proxy.call_method("Close", &());
    }
}

/// Calls a portal method that answers through a Request object, and waits for its Response.
/// The results when the user or the desktop accepted, `None` otherwise.
fn request(
    bus: &Connection,
    call: impl FnOnce(&Proxy<'_>) -> zbus::Result<OwnedObjectPath>,
) -> Option<HashMap<String, OwnedValue>> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .ok()?
        .member("Response")
        .ok()?
        .build();
    let responses = MessageIterator::for_match_rule(rule, bus, None).ok()?;
    let portal = Proxy::new(bus, DESKTOP, DESKTOP_PATH, INTERFACE).ok()?;
    let request = call(&portal).ok()?;
    for message in responses.flatten() {
        let header = message.header();
        if header.path().map(|path| path.as_str()) != Some(request.as_str()) {
            continue;
        }
        let (response, results) = message
            .body()
            .deserialize::<(u32, HashMap<String, OwnedValue>)>()
            .ok()?;
        return (response == 0).then_some(results);
    }
    None
}

fn follow_activations(
    bus: &Connection,
    session: &Mutex<Option<OwnedObjectPath>>,
    on_event: &(dyn Fn(HotkeyEvent) + Send + Sync),
) {
    let Ok(rule) = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(INTERFACE)
        .and_then(|rule| rule.member("Activated"))
        .map(|rule| rule.build())
    else {
        return;
    };
    let Ok(signals) = MessageIterator::for_match_rule(rule, bus, None) else {
        return;
    };
    for message in signals.flatten() {
        let Ok((handle, id, _, _)) =
            message
                .body()
                .deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if session.lock().unwrap().as_ref() != Some(&handle) {
            continue;
        }
        if let Some(slot) = slot_of(&id) {
            on_event(HotkeyEvent::Pressed(slot));
        }
    }
}

fn slot_of(id: &str) -> Option<HotkeySlot> {
    match id {
        "tex" => Some(HotkeySlot::Tex),
        "history" => Some(HotkeySlot::History),
        _ => None,
    }
}

/// `CreateSession`'s session handle: a string in the portal's documentation, an object path
/// in some desktops.
pub(crate) fn session_handle(results: &HashMap<String, OwnedValue>) -> Option<OwnedObjectPath> {
    let value = results.get("session_handle")?;
    if let Ok(text) = value.downcast_ref::<&str>() {
        return ObjectPath::try_from(text).ok().map(Into::into);
    }
    value.downcast_ref::<ObjectPath<'_>>().ok().map(Into::into)
}

/// The `BindShortcuts` list: an id, a description and the preferred trigger for each slot
/// that is wanted.
pub(crate) fn shortcut_list(
    wanted: &Wanted,
) -> Vec<(&'static str, HashMap<&'static str, Value<'_>>)> {
    IDS.iter()
        .zip(wanted)
        .filter_map(|(id, wanted)| {
            let (label, trigger) = wanted.as_ref()?;
            let options = HashMap::from([
                ("description", Value::from(label.as_str())),
                ("preferred_trigger", Value::from(trigger.as_str())),
            ]);
            Some((*id, options))
        })
        .collect()
}

/// The keys each slot got, as the desktop describes them. `None` for a shortcut it did not
/// bind or left without keys.
pub(crate) fn triggers(bound: &[(String, HashMap<String, OwnedValue>)]) -> [Option<String>; 2] {
    IDS.map(|id| {
        let (_, options) = bound.iter().find(|(name, _)| name == id)?;
        let trigger = options
            .get("trigger_description")?
            .downcast_ref::<&str>()
            .ok()?;
        (!trigger.is_empty()).then(|| trigger.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(value: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(value).unwrap()
    }

    #[test]
    fn only_wanted_shortcuts_are_listed() {
        let wanted: Wanted = [Some(("Open Tex".into(), "ALT+space".into())), None];
        let list = shortcut_list(&wanted);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "tex");
        assert_eq!(list[0].1["description"], Value::from("Open Tex"));
        assert_eq!(list[0].1["preferred_trigger"], Value::from("ALT+space"));
        let both: Wanted = [
            Some(("Open Tex".into(), "ALT+space".into())),
            Some(("Clipboard history".into(), "ALT+v".into())),
        ];
        let ids: Vec<&str> = shortcut_list(&both).iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["tex", "history"]);
    }

    #[test]
    fn bound_triggers_come_back_per_slot() {
        let shortcut = |id: &str, trigger: &str| {
            (
                id.to_owned(),
                HashMap::from([(
                    "trigger_description".to_owned(),
                    owned(Value::from(trigger)),
                )]),
            )
        };
        let bound = [shortcut("history", "Meta+V"), shortcut("tex", "Alt+Space")];
        assert_eq!(
            triggers(&bound),
            [Some("Alt+Space".to_owned()), Some("Meta+V".to_owned())]
        );
        assert_eq!(triggers(&[shortcut("tex", "")]), [None, None]);
        assert_eq!(triggers(&[]), [None, None]);
    }

    #[test]
    fn the_session_handle_may_be_a_string_or_a_path() {
        let path = "/org/freedesktop/portal/desktop/session/1_42/tex1";
        let text = HashMap::from([("session_handle".to_owned(), owned(Value::from(path)))]);
        assert_eq!(session_handle(&text).unwrap().as_str(), path);
        let object = HashMap::from([(
            "session_handle".to_owned(),
            owned(Value::from(ObjectPath::try_from(path).unwrap())),
        )]);
        assert_eq!(session_handle(&object).unwrap().as_str(), path);
        assert_eq!(session_handle(&HashMap::new()), None);
    }

    #[test]
    fn activations_name_their_slot() {
        assert_eq!(slot_of("tex"), Some(HotkeySlot::Tex));
        assert_eq!(slot_of("history"), Some(HotkeySlot::History));
        assert_eq!(slot_of("other"), None);
    }
}
