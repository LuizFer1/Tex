mod document;
mod encoding;
mod kind;
mod lines;
mod markdown;

pub use document::{
    Body, Document, EDIT_LIMIT, LoadError, PREVIEW_BYTES, Probe, Stamp, TEXT_LIMIT, probe,
};
pub use encoding::{Decoded, Encoding, LineEnding, decode, detect, encode, fits};
pub use kind::{CORE_EXTENSIONS, Kind, SNIFF_LEN, Sniff, sniff};
pub use lines::{LineIndex, MAX_LINE_BYTES};
pub use markdown::{Block, blocks};
