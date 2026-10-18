mod encoding;
mod kind;
mod lines;
mod markdown;

pub use encoding::{Decoded, Encoding, LineEnding, decode, detect, encode, fits};
pub use kind::{CORE_EXTENSIONS, Kind, SNIFF_LEN, Sniff, sniff};
pub use lines::{LineIndex, MAX_LINE_BYTES};
pub use markdown::{Block, blocks};
