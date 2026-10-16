use tex_index::{Attributes, Record, VolumeIndex, record_from_frn};

pub const REASON_FILE_DELETE: u32 = 0x0000_0200;

const ATTRIBUTE_HIDDEN: u32 = 0x2;
const ATTRIBUTE_SYSTEM: u32 = 0x4;
const ATTRIBUTE_DIRECTORY: u32 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnRecord {
    pub record: Record,
    pub parent: Record,
    pub reason: u32,
    pub attributes: Attributes,
    pub name: String,
}

/// Splits the output of FSCTL_ENUM_USN_DATA or FSCTL_READ_USN_JOURNAL into its leading
/// continuation value (next file reference or next USN) and the records that follow.
/// Records with ids above 32 bits are skipped; parsing stops at the first malformed record.
pub fn parse_batch(buffer: &[u8]) -> Option<(u64, Vec<UsnRecord>)> {
    let next = u64_at(buffer, 0)?;
    let mut records = Vec::new();
    let mut rest = &buffer[8..];
    while let Some(length) = u32_at(rest, 0) {
        let length = length as usize;
        if length < 8 || length > rest.len() {
            break;
        }
        if let Some(record) = parse_record(&rest[..length]) {
            records.push(record);
        }
        rest = &rest[length..];
    }
    Some((next, records))
}

/// Deletes remove the entry; every other change creates or updates it with the record's current name and parent.
pub fn apply(index: &mut VolumeIndex, record: &UsnRecord) {
    if record.reason & REASON_FILE_DELETE != 0 {
        index.remove(record.record);
    } else {
        index.upsert(
            record.record,
            record.parent,
            &record.name,
            record.attributes,
        );
    }
}

fn parse_record(bytes: &[u8]) -> Option<UsnRecord> {
    let (frn, parent, reason_at, attributes_at, name_at) = match u16_at(bytes, 4)? {
        2 => (u64_at(bytes, 8)?, u64_at(bytes, 16)?, 40, 52, 56),
        3 => {
            if u64_at(bytes, 16)? != 0 || u64_at(bytes, 32)? != 0 {
                return None;
            }
            (u64_at(bytes, 8)?, u64_at(bytes, 24)?, 56, 68, 72)
        }
        _ => return None,
    };
    let name_len = u16_at(bytes, name_at)? as usize;
    let name_offset = u16_at(bytes, name_at + 2)? as usize;
    let units: Vec<u16> = bytes
        .get(name_offset..name_offset + name_len)?
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let attributes = u32_at(bytes, attributes_at)?;
    Some(UsnRecord {
        record: record_from_frn(frn)?,
        parent: record_from_frn(parent)?,
        reason: u32_at(bytes, reason_at)?,
        attributes: Attributes {
            dir: attributes & ATTRIBUTE_DIRECTORY != 0,
            hidden: attributes & ATTRIBUTE_HIDDEN != 0,
            system: attributes & ATTRIBUTE_SYSTEM != 0,
        },
        name: String::from_utf16_lossy(&units),
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_index::{ROOT, VolumeIndex};

    const FILE_CREATE: u32 = 0x100;

    fn utf16(name: &str) -> Vec<u8> {
        name.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn v2(frn: u64, parent: u64, reason: u32, attributes: u32, name: &str) -> Vec<u8> {
        let name = utf16(name);
        let length = (60 + name.len()).next_multiple_of(8);
        let mut bytes = vec![0u8; length];
        bytes[0..4].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&frn.to_le_bytes());
        bytes[16..24].copy_from_slice(&parent.to_le_bytes());
        bytes[40..44].copy_from_slice(&reason.to_le_bytes());
        bytes[52..56].copy_from_slice(&attributes.to_le_bytes());
        bytes[56..58].copy_from_slice(&(name.len() as u16).to_le_bytes());
        bytes[58..60].copy_from_slice(&60u16.to_le_bytes());
        bytes[60..60 + name.len()].copy_from_slice(&name);
        bytes
    }

    fn v3(frn: u64, frn_high: u64, parent: u64, reason: u32, name: &str) -> Vec<u8> {
        let name = utf16(name);
        let length = (76 + name.len()).next_multiple_of(8);
        let mut bytes = vec![0u8; length];
        bytes[0..4].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[4..6].copy_from_slice(&3u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&frn.to_le_bytes());
        bytes[16..24].copy_from_slice(&frn_high.to_le_bytes());
        bytes[24..32].copy_from_slice(&parent.to_le_bytes());
        bytes[56..60].copy_from_slice(&reason.to_le_bytes());
        bytes[72..74].copy_from_slice(&(name.len() as u16).to_le_bytes());
        bytes[74..76].copy_from_slice(&76u16.to_le_bytes());
        bytes[76..76 + name.len()].copy_from_slice(&name);
        bytes
    }

    fn batch(next: u64, records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = next.to_le_bytes().to_vec();
        for record in records {
            bytes.extend_from_slice(record);
        }
        bytes
    }

    #[test]
    fn parses_v2_records() {
        let bytes = batch(
            99,
            &[
                v2(
                    0x0001_0000_0000_0020,
                    0x0005_0000_0000_0005,
                    FILE_CREATE,
                    0x10,
                    "Projetos",
                ),
                v2(33, 32, 0, 0x2 | 0x20, "Relatório.pdf"),
            ],
        );
        let (next, records) = parse_batch(&bytes).unwrap();
        assert_eq!(next, 99);
        assert_eq!(
            records,
            [
                UsnRecord {
                    record: 32,
                    parent: ROOT,
                    reason: FILE_CREATE,
                    attributes: Attributes {
                        dir: true,
                        hidden: false,
                        system: false
                    },
                    name: "Projetos".into(),
                },
                UsnRecord {
                    record: 33,
                    parent: 32,
                    reason: 0,
                    attributes: Attributes {
                        dir: false,
                        hidden: true,
                        system: false
                    },
                    name: "Relatório.pdf".into(),
                },
            ]
        );
    }

    #[test]
    fn parses_v3_records_with_small_ids_only() {
        let bytes = batch(
            1,
            &[v3(40, 0, 5, 0, "small.txt"), v3(41, 7, 5, 0, "refs.txt")],
        );
        let (_, records) = parse_batch(&bytes).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record, 40);
        assert_eq!(records[0].name, "small.txt");
    }

    #[test]
    fn stops_at_a_truncated_record() {
        let mut bytes = batch(1, &[v2(40, 5, 0, 0, "ok.txt"), v2(41, 5, 0, 0, "cut.txt")]);
        bytes.truncate(bytes.len() - 10);
        let (_, records) = parse_batch(&bytes).unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn short_buffers_are_rejected() {
        assert!(parse_batch(&[1, 2, 3]).is_none());
    }

    #[test]
    fn apply_creates_renames_and_deletes() {
        let mut index = VolumeIndex::new('C');
        let dir = Attributes {
            dir: true,
            ..Attributes::default()
        };
        let file = Attributes::default();
        let record = |record, parent, reason, attributes, name: &str| UsnRecord {
            record,
            parent,
            reason,
            attributes,
            name: name.into(),
        };
        apply(&mut index, &record(20, ROOT, FILE_CREATE, dir, "a"));
        apply(&mut index, &record(21, ROOT, FILE_CREATE, dir, "b"));
        apply(&mut index, &record(30, 20, FILE_CREATE, file, "draft.txt"));
        apply(&mut index, &record(30, 21, 0x2000, file, "final.txt"));
        assert_eq!(index.path(30).as_deref(), Some(r"C:\b\final.txt"));
        apply(
            &mut index,
            &record(30, 21, REASON_FILE_DELETE, file, "final.txt"),
        );
        assert_eq!(index.path(30), None);
    }
}
