mod capture;
mod crypto;
mod image;
mod retention;
mod store;

pub use capture::{
    CaptureSettings, ClipFacts, Decision, LastCopy, REPEAT_WINDOW_MS, Skip, fits,
    image_fingerprint, is_repeat, same_app, should_capture, text_fingerprint,
};
pub use crypto::{KEY_LEN, Key, new_key, open, seal};
pub use image::{Rgba, decode_png, dib_to_rgba, encode_png, rgba_to_dib, thumbnail};
pub use retention::{DAY_MS, Retention, RowFacts, expired, next_expiry, storage_victims};
pub use store::{
    Content, EditError, HistoryFilter, Kind, NewEntry, OpenError, Preview, Store, Summary,
};
