use std::fmt;
use std::io::{self, Read, Write};

use crate::entry::Entry;
use crate::volume::VolumeIndex;

const MAGIC: [u8; 8] = *b"TEXINDEX";
const VERSION: u32 = 1;
const NO_RECORD: u32 = u32::MAX;
#[cfg(test)]
const HEADER_LEN: usize = 56;
const ENTRY_LEN: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub serial: u32,
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

/// Writes the header followed by every entry (11 bytes each) and the name buffer.
/// Wrap `out` in a `BufWriter` when writing to a file.
pub fn write_snapshot(
    out: &mut impl Write,
    index: &VolumeIndex,
    header: &SnapshotHeader,
) -> io::Result<()> {
    let (entries, names) = index.raw_parts();
    out.write_all(&MAGIC)?;
    out.write_all(&VERSION.to_le_bytes())?;
    out.write_all(&(index.letter() as u32).to_le_bytes())?;
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
    let letter = char::from_u32(u32::from_le_bytes(read_array(input)?))
        .filter(char::is_ascii_uppercase)
        .ok_or(SnapshotError::Corrupt("drive letter"))?;
    let header = SnapshotHeader {
        serial: u32::from_le_bytes(read_array(input)?),
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

    let index = VolumeIndex::from_raw_parts(letter, entries, names, windows_dir)
        .map_err(SnapshotError::Corrupt)?;
    Ok((index, header))
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
        let mut volume = VolumeIndex::new('D');
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
        assert_eq!(restored.letter(), 'D');
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
    fn rejects_truncated_files() {
        let mut bytes = encode();
        bytes.truncate(bytes.len() - 3);
        assert!(read_snapshot(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn rejects_names_outside_the_buffer() {
        let mut bytes = encode();
        let name_len_at = HEADER_LEN + 21 * ENTRY_LEN + 8;
        bytes[name_len_at..name_len_at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            read_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Corrupt(_))
        ));
    }
}
