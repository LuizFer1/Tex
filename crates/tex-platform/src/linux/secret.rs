use std::collections::HashMap;
use std::io;

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::dbus::session_bus;

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const ATTRIBUTE: &str = "application";
const APP: &str = "io.github.LuizFer1.Tex";
const LABEL: &str = "Tex clipboard history key";
/// What [`protect`] returns when the key went to the Secret Service.
const IN_KEYRING: &[u8] = b"tex-key:secret-service\n";
/// What precedes the key itself when there is no Secret Service.
const IN_FILE: &[u8] = b"tex-key:file\n";

/// The Secret Service's `Secret` struct: session, parameters, value and content type.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// Where [`protect`] left the key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sealed<'a> {
    Keyring,
    File(&'a [u8]),
}

pub(crate) fn sealed(bytes: &[u8]) -> Option<Sealed<'_>> {
    if bytes == IN_KEYRING {
        return Some(Sealed::Keyring);
    }
    bytes.strip_prefix(IN_FILE).map(Sealed::File)
}

/// The text kept in a file only this user can read when there is no Secret Service.
pub(crate) fn in_file(data: &[u8]) -> Vec<u8> {
    [IN_FILE, data].concat()
}

/// Puts `data` (the history key) in the Secret Service and returns a reference to it. Without
/// a Secret Service (a bare window manager) the result carries `data` itself, which the caller
/// keeps in a file only this user can read.
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    match store(data) {
        Ok(()) => Ok(IN_KEYRING.to_vec()),
        Err(error) => {
            eprintln!(
                "clipboard key: no Secret Service ({error}); keeping it in a file only this user can read"
            );
            Ok(in_file(data))
        }
    }
}

/// The key behind what [`protect`] returned. A Secret Service that cannot be reached or stays
/// locked is `NotConnected`, so the caller keeps the history for the next start.
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    match sealed(data) {
        Some(Sealed::File(key)) => Ok(key.to_vec()),
        Some(Sealed::Keyring) => load(),
        None => Err(io::Error::new(io::ErrorKind::InvalidData, "not a Tex key")),
    }
}

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([(ATTRIBUTE, APP)])
}

fn failure(message: &str) -> zbus::Error {
    zbus::Error::Failure(message.to_owned())
}

fn service(bus: &Connection) -> zbus::Result<Proxy<'static>> {
    Proxy::new(bus, SERVICE, SERVICE_PATH, "org.freedesktop.Secret.Service")
}

/// A session that moves secrets unencrypted: the session bus only reaches this user's
/// processes, which can read the file fallback anyway.
fn open_session(service: &Proxy<'_>) -> zbus::Result<OwnedObjectPath> {
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service.call("OpenSession", &("plain", Value::from("")))?;
    Ok(session)
}

fn store(data: &[u8]) -> zbus::Result<()> {
    let bus = session_bus().ok_or_else(|| failure("no session bus"))?;
    let service = service(&bus)?;
    let session = open_session(&service)?;
    let collection: OwnedObjectPath = service.call("ReadAlias", &("default",))?;
    if collection.as_str() == "/" {
        return Err(failure("no default keyring"));
    }
    unlock(&bus, &service, vec![collection.clone()])?;
    let collection = Proxy::new(
        &bus,
        SERVICE,
        collection,
        "org.freedesktop.Secret.Collection",
    )?;
    let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
    properties.insert("org.freedesktop.Secret.Item.Label", Value::from(LABEL));
    properties.insert(
        "org.freedesktop.Secret.Item.Attributes",
        Value::from(attributes()),
    );
    let secret: Secret = (
        session,
        Vec::new(),
        data.to_vec(),
        "application/octet-stream".to_owned(),
    );
    let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
        collection.call("CreateItem", &(properties, secret, true))?;
    run_prompt(&bus, &prompt)
}

fn load() -> io::Result<Vec<u8>> {
    let unreachable = |error: zbus::Error| io::Error::new(io::ErrorKind::NotConnected, error);
    let bus = session_bus()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no session bus"))?;
    let service = service(&bus).map_err(unreachable)?;
    let session = open_session(&service).map_err(unreachable)?;
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
        .call("SearchItems", &(attributes(),))
        .map_err(unreachable)?;
    let item = match unlocked.into_iter().next() {
        Some(item) => item,
        None => {
            let item = locked.into_iter().next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "the key is not in the keyring")
            })?;
            unlock(&bus, &service, vec![item.clone()]).map_err(unreachable)?;
            item
        }
    };
    let item =
        Proxy::new(&bus, SERVICE, item, "org.freedesktop.Secret.Item").map_err(unreachable)?;
    let (_, _, value, _): Secret = item.call("GetSecret", &(session,)).map_err(unreachable)?;
    Ok(value)
}

fn unlock(
    bus: &Connection,
    service: &Proxy<'_>,
    objects: Vec<OwnedObjectPath>,
) -> zbus::Result<()> {
    let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service.call("Unlock", &(objects,))?;
    run_prompt(bus, &prompt)
}

/// Shows the keyring's prompt (to unlock or to confirm) and waits for the user. `/` means
/// there is nothing to ask.
fn run_prompt(bus: &Connection, prompt: &OwnedObjectPath) -> zbus::Result<()> {
    if prompt.as_str() == "/" {
        return Ok(());
    }
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.Secret.Prompt")?
        .member("Completed")?
        .path(prompt.as_str())?
        .build();
    let completed = MessageIterator::for_match_rule(rule, bus, None)?;
    let proxy = Proxy::new(
        bus,
        SERVICE,
        prompt.as_str(),
        "org.freedesktop.Secret.Prompt",
    )?;
    proxy.call_method("Prompt", &("",))?;
    if let Some(message) = completed.flatten().next() {
        let (dismissed, _): (bool, OwnedValue) = message.body().deserialize()?;
        return if dismissed {
            Err(failure("the keyring prompt was dismissed"))
        } else {
            Ok(())
        };
    }
    Err(failure("the keyring prompt went away"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyring_reference_is_recognized() {
        assert_eq!(sealed(IN_KEYRING), Some(Sealed::Keyring));
        assert_eq!(sealed(b"tex-key:secret-service\nextra"), None);
    }

    #[test]
    fn a_key_in_a_file_round_trips() {
        let key = [7u8; 32];
        let stored = in_file(&key);
        assert!(stored.starts_with(b"tex-key:file\n"));
        assert_eq!(sealed(&stored), Some(Sealed::File(&key[..])));
        assert_eq!(unprotect(&stored).unwrap(), key);
    }

    #[test]
    fn anything_else_is_not_a_key() {
        assert_eq!(sealed(b"garbage"), None);
        assert_eq!(
            unprotect(b"garbage").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
