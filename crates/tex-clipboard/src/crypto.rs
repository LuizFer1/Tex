use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

#[derive(Clone)]
pub struct Key([u8; KEY_LEN]);

impl Key {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Key {
        Key(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.0.into())
    }
}

pub fn new_key() -> Key {
    Key(chacha20poly1305::Key::generate().into())
}

/// `nonce || ciphertext`, with a fresh random nonce.
pub fn seal(key: &Key, plain: &[u8]) -> Vec<u8> {
    let nonce = XNonce::generate();
    let mut out = nonce.to_vec();
    out.extend(
        key.cipher()
            .encrypt(&nonce, plain)
            .expect("encryption does not fail"),
    );
    out
}

/// `None` when the data was not sealed with this key or was changed.
pub fn open(key: &Key, sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < NONCE_LEN {
        return None;
    }
    let (nonce, data) = sealed.split_at(NONCE_LEN);
    let nonce = XNonce::try_from(nonce).ok()?;
    key.cipher().decrypt(&nonce, data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = new_key();
        let sealed = seal(&key, "olá, mundo".as_bytes());
        assert_eq!(
            open(&key, &sealed).as_deref(),
            Some("olá, mundo".as_bytes())
        );
    }

    #[test]
    fn same_text_seals_differently() {
        let key = new_key();
        assert_ne!(seal(&key, b"same"), seal(&key, b"same"));
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&new_key(), b"secret");
        assert_eq!(open(&new_key(), &sealed), None);
    }

    #[test]
    fn changed_byte_fails() {
        let key = new_key();
        let mut sealed = seal(&key, b"secret");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(open(&key, &sealed), None);
        let mut sealed = seal(&key, b"secret");
        sealed[0] ^= 1;
        assert_eq!(open(&key, &sealed), None);
    }

    #[test]
    fn short_input_fails() {
        let key = new_key();
        assert_eq!(open(&key, &[]), None);
        assert_eq!(open(&key, &[0; 23]), None);
        assert_eq!(open(&key, &[0; 24]), None);
    }

    #[test]
    fn keys_differ_and_round_trip_through_bytes() {
        let key = new_key();
        assert_ne!(key.as_bytes(), new_key().as_bytes());
        let copy = Key::from_bytes(*key.as_bytes());
        assert_eq!(open(&copy, &seal(&key, b"x")).as_deref(), Some(&b"x"[..]));
    }
}
