const METADATA_LEN: usize = 24;
const INFO_HEADER_LEN: usize = 4;
const INFO_DFID_NAME: u8 = 2;
const Q_OVERFLOW: u64 = 0x0000_4000;

/// The name the kernel gives an event on a folder itself (such as `FAN_ATTRIB` on a folder): the
/// handle is then the folder's own, so the change concerns that folder and not one of its entries.
pub const ITSELF: &str = ".";

/// One event read from a fanotify group that reports `FAN_REPORT_DFID_NAME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Something happened to the entry `name` of the folder with handle `dir` (handle type then
    /// handle bytes, as `name_to_handle_at` gives them) on the file system `fsid`. A name of
    /// `ITSELF` means the folder `dir` itself.
    Change {
        fsid: u64,
        dir: Vec<u8>,
        name: String,
    },
    /// The kernel dropped events: the file systems must be scanned again.
    Overflow,
}

/// Reads the events in `buffer` (what one `read` of the fanotify descriptor returned). Events
/// without a folder and a UTF-8 name are skipped; a truncated event ends the list.
pub fn parse_events(buffer: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    let mut offset = 0;
    while let Some(header) = buffer.get(offset..offset + METADATA_LEN) {
        let event_len = le32(header, 0).unwrap_or(0) as usize;
        let metadata_len = usize::from(le16(header, 6).unwrap_or(0));
        let mask = le64(header, 8).unwrap_or(0);
        if metadata_len < METADATA_LEN || event_len < metadata_len {
            break;
        }
        let Some(event) = buffer.get(offset..offset + event_len) else {
            break;
        };
        if mask & Q_OVERFLOW != 0 {
            events.push(Event::Overflow);
        } else if let Some(change) = change(&event[metadata_len..]) {
            events.push(change);
        }
        offset += event_len;
    }
    events
}

/// The directory-and-name record among the information records of one event.
fn change(mut records: &[u8]) -> Option<Event> {
    while records.len() >= INFO_HEADER_LEN {
        let kind = records[0];
        let len = usize::from(le16(records, 2)?);
        if len < INFO_HEADER_LEN || len > records.len() {
            return None;
        }
        if kind == INFO_DFID_NAME {
            return dfid_name(&records[..len]);
        }
        records = &records[len..];
    }
    None
}

/// `header (4) | fsid (8) | handle_bytes (4) | handle_type (4) | handle | name NUL [padding]`.
fn dfid_name(record: &[u8]) -> Option<Event> {
    let fsid = u64::from(le32(record, 4)?) | (u64::from(le32(record, 8)?) << 32);
    let handle_len = le32(record, 12)? as usize;
    let handle_end = 20usize.checked_add(handle_len)?;
    let mut dir = record.get(16..20)?.to_vec();
    dir.extend_from_slice(record.get(20..handle_end)?);
    let rest = record.get(handle_end..)?;
    let name = std::str::from_utf8(&rest[..rest.iter().position(|&byte| byte == 0)?]).ok()?;
    if name.is_empty() {
        return None;
    }
    Some(Event::Change {
        fsid,
        dir,
        name: name.to_owned(),
    })
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATE: u64 = 0x100;

    fn info(kind: u8, fsid: [u32; 2], handle_type: i32, handle: &[u8], name: &str) -> Vec<u8> {
        let mut record = vec![kind, 0, 0, 0];
        record.extend(fsid[0].to_le_bytes());
        record.extend(fsid[1].to_le_bytes());
        record.extend((handle.len() as u32).to_le_bytes());
        record.extend(handle_type.to_le_bytes());
        record.extend(handle);
        record.extend(name.as_bytes());
        record.push(0);
        while record.len() % 4 != 0 {
            record.push(0);
        }
        let len = record.len() as u16;
        record[2..4].copy_from_slice(&len.to_le_bytes());
        record
    }

    fn event(mask: u64, records: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = records.concat();
        let mut bytes = Vec::new();
        bytes.extend(((METADATA_LEN + body.len()) as u32).to_le_bytes());
        bytes.extend([3, 0]);
        bytes.extend((METADATA_LEN as u16).to_le_bytes());
        bytes.extend(mask.to_le_bytes());
        bytes.extend((-1i32).to_le_bytes());
        bytes.extend(1234i32.to_le_bytes());
        bytes.extend(body);
        bytes
    }

    fn dir_key(handle_type: i32, handle: &[u8]) -> Vec<u8> {
        let mut key = handle_type.to_le_bytes().to_vec();
        key.extend(handle);
        key
    }

    #[test]
    fn reads_folder_and_name_of_each_event() {
        let mut buffer = event(
            CREATE,
            &[info(
                INFO_DFID_NAME,
                [0x11, 0x22],
                1,
                &[1, 2, 3, 4, 5, 6, 7, 8],
                "notes.txt",
            )],
        );
        buffer.extend(event(
            CREATE,
            &[info(
                INFO_DFID_NAME,
                [0x11, 0x22],
                1,
                &[9, 9, 9, 9],
                "relatório",
            )],
        ));
        assert_eq!(
            parse_events(&buffer),
            [
                Event::Change {
                    fsid: 0x0000_0022_0000_0011,
                    dir: dir_key(1, &[1, 2, 3, 4, 5, 6, 7, 8]),
                    name: "notes.txt".into(),
                },
                Event::Change {
                    fsid: 0x0000_0022_0000_0011,
                    dir: dir_key(1, &[9, 9, 9, 9]),
                    name: "relatório".into(),
                },
            ]
        );
    }

    #[test]
    fn overflow_is_reported() {
        assert_eq!(parse_events(&event(Q_OVERFLOW, &[])), [Event::Overflow]);
    }

    #[test]
    fn folder_itself_is_reported_with_the_dot_name() {
        let dot = event(
            CREATE,
            &[info(INFO_DFID_NAME, [1, 0], 1, &[1, 2, 3, 4], ITSELF)],
        );
        assert_eq!(
            parse_events(&dot),
            [Event::Change {
                fsid: 1,
                dir: dir_key(1, &[1, 2, 3, 4]),
                name: ITSELF.into(),
            }]
        );
    }

    #[test]
    fn skips_events_without_a_folder_and_name() {
        let other_kind = event(CREATE, &[info(1, [1, 0], 1, &[1, 2, 3, 4], "")]);
        let mut buffer = other_kind;
        buffer.extend(event(
            CREATE,
            &[
                info(1, [1, 0], 1, &[5, 5, 5, 5], ""),
                info(INFO_DFID_NAME, [1, 0], 1, &[1, 2, 3, 4], "a"),
            ],
        ));
        assert_eq!(
            parse_events(&buffer),
            [Event::Change {
                fsid: 1,
                dir: dir_key(1, &[1, 2, 3, 4]),
                name: "a".into(),
            }]
        );
    }

    #[test]
    fn truncated_buffers_stop_quietly() {
        let buffer = event(
            CREATE,
            &[info(INFO_DFID_NAME, [1, 0], 1, &[1, 2, 3, 4], "a")],
        );
        for len in 0..buffer.len() {
            assert!(parse_events(&buffer[..len]).is_empty());
        }
        let mut broken = buffer.clone();
        broken[0..4].copy_from_slice(&3u32.to_le_bytes());
        assert!(parse_events(&broken).is_empty());
    }
}
