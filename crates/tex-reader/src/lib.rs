mod encoding;
mod kind;

pub use encoding::{Decoded, Encoding, LineEnding, decode, detect, encode, fits};
pub use kind::{CORE_EXTENSIONS, Kind, SNIFF_LEN, Sniff, sniff};
