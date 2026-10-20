mod document;
mod encoding;
mod extensions;
mod images;
mod kind;
mod lines;
mod markdown;
mod runner;

pub use document::{
    Body, Document, EDIT_LIMIT, LoadError, PREVIEW_BYTES, Probe, Stamp, TEXT_LIMIT, probe,
};
pub use encoding::{Encoding, LineEnding, detect, encode, fits};
pub use extensions::{
    Broken, Catalog, Extension, Input, Job, MANIFEST, Tool, ToolOutput, Viewer, ViewerOutput, job,
};
pub use images::{Decoded, Frame, MAX_FILE_BYTES, Reason, Source, decode, resolve};
pub use kind::{CORE_EXTENSIONS, Kind, SNIFF_LEN, Sniff, sniff};
pub use lines::{LineIndex, MAX_LINE_BYTES};
pub use markdown::{Block, blocks};
pub use runner::{Cancel, Inbox, LIMITS, Limits, Message, RunError, channel, run};
