pub mod account;
pub mod engine;
pub mod log;
pub mod pipe;
mod security;
pub mod store;
pub mod usn;

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
