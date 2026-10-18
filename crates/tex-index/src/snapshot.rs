use std::fmt;
use std::io::{self, Read, Write};

use crate::access::Owner;
use crate::entry::{Entry, Record};
use crate::volume::VolumeIndex;

const MAGIC: [u8; 8] = *b"TEXINDEX";
const VERSION: u32 = 3;
const NO_RECORD: u32 = u32::MAX;
const MAX_ROOT_LEN: u32 = 4096;
#[cfg(test)]
const FIXED_HEADER_LEN: usize = 60;
const ENTRY_LEN: usize = 11;
const OWNER_LEN: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub serial: u64,
    pub journal_id: u64,
    pub next_usn: i64,
}

#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    NotASnapshot,
    Version(u32),
    Corrupt(&'static str),
}

impl From<io::Error> for SnapshotError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "snapshot I/O error: {error}"),
            Self::NotASnapshot => f.write_str("file is not a Tex index snapshot"),
            Self::Version(version) => {
                write!(f, "snapshot format version {version} is not supported")
            }
            Self::Corrupt(what) => write!(f, "snapshot is corrupt: {what}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Writes the header (with the volume's root), every entry (11 bytes each), the name buffer and
/// the owners of the directories (14 bytes each). Wrap `out` in a `BufWriter` when writing to a file.
pub fn write_snapshot(
    out: &mut impl Write,
    index: &VolumeIndex,
    header: &SnapshotHeader,
) -> io::Result<()> {
    let (entries, names) = index.raw_parts();
    let root = index.root().as_bytes();
    out.write_all(&MAGIC)?;
    out.write_all(&VERSION.to_le_bytes())?;
    out.write_all(&(root.len() as u32).to_le_bytes())?;
    out.write_all(root)?;
    out.write_all(&header.serial.to_le_bytes())?;
    out.write_all(&header.journal_id.to_le_bytes())?;
    out.write_all(&header.next_usn.to_le_bytes())?;
    out.write_all(&index.windows_dir().unwrap_or(NO_RECORD).to_le_bytes())?;
    out.write_all(&(entries.len() as u64).to_le_bytes())?;
    out.write_all(&(names.len() as u64).to_le_bytes())?;
    for entry in entries {
        out.write_all(&entry.parent.to_le_bytes())?;
        out.write_all(&entry.name_offset.to_le_bytes())?;
        out.write_all(&entry.name_len.to_le_bytes())?;
        out.write_all(&[entry.flags])?;
    }
    out.write_all(names)?;
    let mut owners: Vec<(Record, Owner)> = index
        .owners()
        .iter()
        .map(|(&record, &owner)| (record, owner))
        .collect();
    owners.sort_unstable_by_key(|&(record, _)| record);
    out.write_all(&(owners.len() as u64).to_le_bytes())?;
    for (record, owner) in owners {
        out.write_all(&record.to_le_bytes())?;
        out.write_all(&owner.uid.to_le_bytes())?;
        out.write_all(&owner.gid.to_le_bytes())?;
        out.write_all(&owner.mode.to_le_bytes())?;
    }
    out.flush()
}

/// Reads a snapshot written by `write_snapshot`. Wrap `input` in a `BufReader` when reading a file.
pub fn read_snapshot(
    input: &mut impl Read,
) -> Result<(VolumeIndex, SnapshotHeader), SnapshotError> {
    if read_array::<8>(input)? != MAGIC {
        return Err(SnapshotError::NotASnapshot);
    }
    let version = u32::from_le_bytes(read_array(input)?);
    if version != VERSION {
        return Err(SnapshotError::Version(version));
    }
    let root_len = u32::from_le_bytes(read_array(input)?);
    if root_len == 0 || root_len > MAX_ROOT_LEN {
        return Err(SnapshotError::Corrupt("root"));
    }
    let mut root = vec![0; root_len as usize];
    input.read_exact(&mut root)?;
    let root = String::from_utf8(root)
        .ok()
        .filter(|root| valid_root(root))
        .ok_or(SnapshotError::Corrupt("root"))?;
    let header = SnapshotHeader {
        serial: u64::from_le_bytes(read_array(input)?),
        journal_id: u64::from_le_bytes(read_array(input)?),
        next_usn: i64::from_le_bytes(read_array(input)?),
    };
    let windows_dir = match u32::from_le_bytes(read_array(input)?) {
        NO_RECORD => None,
        record => Some(record),
    };
    let entry_count = u64::from_le_bytes(read_array(input)?);
    let name_len = u64::from_le_bytes(read_array(input)?);
    if entry_count > u64::from(u32::MAX) {
        return Err(SnapshotError::Corrupt("entry count"));
    }
    if name_len > u64::from(u32::MAX) {
        return Err(SnapshotError::Corrupt("name buffer size"));
    }

    let mut entries = Vec::with_capacity((entry_count as usize).min(1 << 20));
    for _ in 0..entry_count {
        let bytes = read_array::<ENTRY_LEN>(input)?;
        entries.push(Entry {
            parent: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            name_offset: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            name_len: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
            flags: bytes[10],
        });
    }
    let mut names = Vec::new();
    input.by_ref().take(name_len).read_to_end(&mut names)?;
    if names.len() as u64 != name_len {
        return Err(SnapshotError::Corrupt("truncated name buffer"));
    }

    let mut index = VolumeIndex::from_raw_parts(&root, entries, names, windows_dir)
        .map_err(SnapshotError::Corrupt)?;
    let owner_count = u64::from_le_bytes(read_array(input)?);
    if owner_count > u64::from(u32::MAX) {
        return Err(SnapshotError::Corrupt("owner count"));
    }
    for _ in 0..owner_count {
        let bytes = read_array::<OWNER_LEN>(input)?;
        let owner = Owner {
            uid: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            gid: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            mode: u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
        };
        index.set_owner(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            Some(owner),
        );
    }
    Ok((index, header))
}

/// A drive such as `C:` or an absolute Unix path.
fn valid_root(root: &str) -> bool {
    let bytes = root.as_bytes();
    root.starts_with('/') || (bytes.len() == 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':')
}

fn read_array<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Attributes, ROOT};

    const FILE: Attributes = Attributes {
        dir: false,
        hidden: false,
        system: false,
    };
    const DIR: Attributes = Attributes {
        dir: true,
        hidden: false,
        system: false,
    };

    fn sample() -> (VolumeIndex, SnapshotHeader) {
        let mut volume = VolumeIndex::new("D:");
        volume.upsert(20, ROOT, "Projetos", DIR);
        volume.upsert(21, 20, "Relatório.pdf", FILE);
        volume.upsert(22, 20, "temp.txt", FILE);
        volume.remove(22);
        volume.upsert(30, ROOT, "Windows", DIR);
        volume.set_windows_dir(Some(30));
        let header = SnapshotHeader {
            serial: 0xDEAD_BEEF,
            journal_id: 7,
            next_usn: 123_456,
        };
        (volume, header)
    }

    fn encode() -> Vec<u8> {
        let (volume, header) = sample();
        let mut bytes = Vec::new();
        write_snapshot(&mut bytes, &volume, &header).unwrap();
        bytes
    }

    #[test]
    fn round_trip_keeps_entries_and_header() {
        let (restored, header) = read_snapshot(&mut encode().as_slice()).unwrap();
        assert_eq!(header, sample().1);
        assert_eq!(restored.root(), "D:");
        assert_eq!(restored.len(), 3);
        assert_eq!(
            restored.path(21).as_deref(),
            Some(r"D:\Projetos\Relatório.pdf")
        );
        assert_eq!(restored.name(22), None);
        assert_eq!(restored.windows_dir(), Some(30));
        assert_eq!(restored.garbage(), "temp.txt".len());
    }

    #[test]
    fn round_trip_keeps_unix_roots_and_wide_serials() {
        let mut volume = VolumeIndex::new("/home");
        volume.upsert(20, ROOT, "luis", DIR);
        volume.upsert(21, 20, "notes.txt", FILE);
        let header = SnapshotHeader {
            serial: 0x1234_5678_9ABC_DEF0,
            journal_id: 0,
            next_usn: 0,
        };
        let mut bytes = Vec::new();
        write_snapshot(&mut bytes, &volume, &header).unwrap();
        let (restored, restored_header) = read_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(restored_header, header);
        assert_eq!(restored.root(), "/home");
        assert_eq!(restored.path(21).as_deref(), Some("/home/luis/notes.txt"));
    }

    #[test]
    fn round_trip_keeps_directory_owners() {
        let mut volume = VolumeIndex::new("/home");
        volume.upsert(20, ROOT, "ana", DIR);
        volume.upsert(21, 20, "notes.txt", FILE);
        volume.set_owner(
            ROOT,
            Some(Owner {
                uid: 0,
                gid: 0,
                mode: 0o755,
            }),
        );
        volume.set_owner(
            20,
            Some(Owner {
                uid: 1000,
                gid: 1000,
                mode: 0o700,
            }),
        );
        let header = SnapshotHeader {
            serial: 9,
            journal_id: 0,
            next_usn: 0,
        };
        let mut bytes = Vec::new();
        write_snapshot(&mut bytes, &volume, &header).unwrap();
        assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
        let (restored, _) = read_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(restored.owner(ROOT).map(|owner| owner.mode), Some(0o755));
        assert_eq!(
            restored.owner(20),
            Some(Owner {
                uid: 1000,
                gid: 1000,
                mode: 0o700,
            })
        );
        assert_eq!(restored.owner(21), None);
    }

    #[test]
    fn rejects_other_files() {
        let result = read_snapshot(&mut b"not a snapshot at all".as_slice());
        assert!(matches!(result, Err(SnapshotError::NotASnapshot)));
    }

    #[test]
    fn rejects_other_versions() {
        let mut bytes = encode();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(
            read_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Version(99))
        ));
    }

    #[test]
    fn rejects_bad_roots() {
        let mut bytes = encode();
        bytes[16] = b'x';
        assert!(matches!(
            read_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Corrupt("root"))
        ));
        let mut bytes = encode();
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            read_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Corrupt("root"))
        ));
    }

    #[test]
    fn rejects_truncated_files() {
        let mut bytes = encode();
        bytes.truncate(bytes.len() - 3);
        assert!(read_snapshot(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn rejects_names_outside_the_buffer() {
        let mut bytes = encode();
        let name_len_at = FIXED_HEADER_LEN + "D:".len() + 21 * ENTRY_LEN + 8;
        bytes[name_len_at..name_len_at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            read_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Corrupt(_))
        ));
    }
}
