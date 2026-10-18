mod apps;
mod dense;
mod entry;
mod matcher;
mod normalize;
mod search;
mod snapshot;
mod volume;

pub use apps::DesktopApp;
pub use dense::DenseIds;
pub use entry::{Attributes, ROOT, Record, record_from_frn};
pub use matcher::{MatchKind, Query};
pub use normalize::fold;
pub use search::{Categories, Filter, Hit, SearchOutcome, SearchRequest, search};
pub use snapshot::{SnapshotError, SnapshotHeader, read_snapshot, write_snapshot};
pub use volume::VolumeIndex;
