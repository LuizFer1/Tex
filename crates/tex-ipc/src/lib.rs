use std::fmt::Display;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const PIPE_NAME: &str = r"\\.\pipe\tex-indexer";
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Search(SearchQuery),
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub id: u64,
    pub query: String,
    pub filter: Filter,
    pub limit: u16,
    pub home: String,
    pub include_hidden: bool,
    pub include_system: bool,
    pub excluded_folders: Vec<String>,
    pub show_apps: bool,
    pub show_files: bool,
    pub show_folders: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    All,
    Files,
    Folders,
    Apps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Results(Results),
    Status(Vec<VolumeStatus>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Results {
    pub id: u64,
    pub hits: Vec<Hit>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub path: String,
    pub is_dir: bool,
    pub is_app: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeStatus {
    pub letter: char,
    pub state: VolumeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeState {
    Indexing { percent: u8 },
    Ready { entries: u32 },
    Failed { reason: String },
}

/// Writes one message as a little-endian u32 length followed by its postcard body.
pub fn write_message(out: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let body = postcard::to_stdvec(message).map_err(invalid_data)?;
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(invalid_data("message exceeds the size limit"));
    }
    out.write_all(&(body.len() as u32).to_le_bytes())?;
    out.write_all(&body)?;
    out.flush()
}

/// Reads one message written by `write_message`.
pub fn read_message<T: DeserializeOwned>(input: &mut impl Read) -> io::Result<T> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_MESSAGE_BYTES {
        return Err(invalid_data(format!(
            "message of {len} bytes exceeds the size limit"
        )));
    }
    let mut body = vec![0; len];
    input.read_exact(&mut body)?;
    postcard::from_bytes(&body).map_err(invalid_data)
}

fn invalid_data(error: impl Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(message: T) {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        let decoded: T = read_message(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, message);
    }

    fn search() -> Request {
        Request::Search(SearchQuery {
            id: 42,
            query: "relatório pdf".into(),
            filter: Filter::Files,
            limit: 50,
            home: r"C:\Users\luis".into(),
            include_hidden: false,
            include_system: true,
            excluded_folders: vec![r"C:\Windows".into()],
            show_apps: true,
            show_files: false,
            show_folders: true,
        })
    }

    #[test]
    fn requests_round_trip() {
        round_trip(search());
        round_trip(Request::Status);
    }

    #[test]
    fn responses_round_trip() {
        round_trip(Response::Results(Results {
            id: 42,
            hits: vec![Hit {
                path: r"C:\Projetos\relatorio.pdf".into(),
                is_dir: false,
                is_app: true,
            }],
            total: 1,
        }));
        round_trip(Response::Status(vec![
            VolumeStatus {
                letter: 'C',
                state: VolumeState::Ready { entries: 1_234_567 },
            },
            VolumeStatus {
                letter: 'D',
                state: VolumeState::Indexing { percent: 45 },
            },
            VolumeStatus {
                letter: 'E',
                state: VolumeState::Failed {
                    reason: "access denied".into(),
                },
            },
        ]));
    }

    struct OneByteAtATime<'a>(&'a [u8]);

    impl Read for OneByteAtATime<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some((first, rest)) = self.0.split_first() else {
                return Ok(0);
            };
            if buf.is_empty() {
                return Ok(0);
            }
            buf[0] = *first;
            self.0 = rest;
            Ok(1)
        }
    }

    #[test]
    fn reads_messages_split_across_reads() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &search()).unwrap();
        write_message(&mut bytes, &Request::Status).unwrap();
        let mut input = OneByteAtATime(&bytes);
        assert_eq!(read_message::<Request>(&mut input).unwrap(), search());
        assert_eq!(
            read_message::<Request>(&mut input).unwrap(),
            Request::Status
        );
    }

    #[test]
    fn rejects_oversized_messages() {
        let bytes = (MAX_MESSAGE_BYTES as u32 + 1).to_le_bytes();
        let error = read_message::<Request>(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn reports_end_of_stream() {
        let error = read_message::<Request>(&mut io::empty()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn rejects_garbage_bodies() {
        let mut bytes = 3u32.to_le_bytes().to_vec();
        bytes.extend([0xFF, 0xFF, 0xFF]);
        let error = read_message::<Request>(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
