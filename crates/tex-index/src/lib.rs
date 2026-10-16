mod entry;
mod matcher;
mod normalize;
mod search;
mod volume;

pub use entry::{Attributes, ROOT, Record, record_from_frn};
pub use matcher::{MatchKind, Query};
pub use normalize::fold;
pub use search::{Filter, Hit, SearchOutcome, SearchRequest, search};
pub use volume::VolumeIndex;
