use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::encoding::{Encoding, LineEnding, decode, decode_body, detect, encode};
use crate::kind::{Kind, Sniff, sniff};
use crate::lines::LineIndex;

pub const TEXT_LIMIT: u64 = 2 * 1024 * 1024;
pub const EDIT_LIMIT: u64 = 128 * 1024;
pub const PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum LoadError {
    Unsupported,
    Binary,
    Io(io::Error),
}

impl From<io::Error> for LoadError {
    fn from(error: io::Error) -> Self {
        LoadError::Io(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl Stamp {
    pub fn of(metadata: &Metadata) -> Stamp {
        Stamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        }
    }

    pub fn read(path: &Path) -> io::Result<Stamp> {
        Ok(Stamp::of(&std::fs::metadata(path)?))
    }
}

/// The first `PREVIEW_BYTES` of a file and what the reader makes of them, before the full load.
#[derive(Debug)]
pub struct Probe {
    pub path: PathBuf,
    pub kind: Kind,
    pub stamp: Stamp,
    pub encoding: Encoding,
    pub head: Vec<u8>,
}

pub fn probe(path: &Path, viewer_exts: &[String]) -> Result<Probe, LoadError> {
    let kind = Kind::from_path(path, viewer_exts).ok_or(LoadError::Unsupported)?;
    let file = File::open(path)?;
    let stamp = Stamp::of(&file.metadata()?);
    let mut head = Vec::with_capacity(PREVIEW_BYTES);
    file.take(PREVIEW_BYTES as u64).read_to_end(&mut head)?;
    let kind = match kind {
        Kind::Unknown if sniff(&head) == Sniff::Binary => return Err(LoadError::Binary),
        Kind::Unknown => Kind::Plain { wrap: true },
        kind => kind,
    };
    let encoding = detect(&head);
    Ok(Probe {
        path: path.to_path_buf(),
        kind,
        stamp,
        encoding,
        head,
    })
}

impl Probe {
    pub fn large(&self) -> bool {
        self.stamp.len > TEXT_LIMIT
    }

    /// The lines of the head, without the last one when the file goes on past it.
    pub fn preview_lines(&self) -> Vec<String> {
        let body = &self.head[self.encoding.bom_len().min(self.head.len())..];
        let text = decode_body(body, self.encoding);
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
            .collect();
        if (self.head.len() as u64) < self.stamp.len && lines.len() > 1 {
            lines.pop();
        }
        lines
    }

    /// Reads the whole file (up to `TEXT_LIMIT`) or indexes its lines; `None` when cancelled.
    pub fn load(self, cancel: &AtomicBool) -> Result<Option<Document>, LoadError> {
        if self.large() {
            let mut file = File::open(&self.path)?;
            let Some(index) = LineIndex::build(&mut file, self.encoding, cancel)? else {
                return Ok(None);
            };
            return Ok(Some(Document {
                path: self.path,
                kind: self.kind,
                encoding: self.encoding,
                line_ending: LineEnding::native(),
                stamp: self.stamp,
                body: Body::Lines(index),
                dirty: false,
            }));
        }
        let decoded = decode(&std::fs::read(&self.path)?);
        Ok(Some(Document {
            path: self.path,
            kind: self.kind,
            encoding: decoded.encoding,
            line_ending: decoded.line_ending,
            stamp: self.stamp,
            body: Body::Text(decoded.text),
            dirty: false,
        }))
    }
}

#[derive(Debug)]
pub enum Body {
    Text(String),
    Lines(LineIndex),
}

#[derive(Debug)]
pub struct Document {
    pub path: PathBuf,
    pub kind: Kind,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub stamp: Stamp,
    pub body: Body,
    pub dirty: bool,
}

impl Document {
    pub fn editable(&self) -> bool {
        matches!(&self.body, Body::Text(text) if text.len() as u64 <= EDIT_LIMIT)
    }

    pub fn changed(&self, current: Stamp) -> bool {
        self.stamp != current
    }

    pub fn set_text(&mut self, text: String) {
        if let Body::Text(current) = &mut self.body
            && *current != text
        {
            *current = text;
            self.dirty = true;
        }
    }

    pub fn save_bytes(&self) -> Option<Vec<u8>> {
        match &self.body {
            Body::Text(text) => Some(encode(text, self.encoding, self.line_ending)),
            Body::Lines(_) => None,
        }
    }

    pub fn saved(&mut self, stamp: Stamp) {
        self.stamp = stamp;
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tex-reader-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn open(path: &Path) -> Document {
        probe(path, &[])
            .unwrap()
            .load(&AtomicBool::new(false))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn small_markdown_loads_as_editable_text() {
        let path = temp("n.md", b"# T\r\nx\r\n");
        let probed = probe(&path, &[]).unwrap();
        assert_eq!(probed.kind, Kind::Markdown);
        assert!(!probed.large());
        let document = open(&path);
        assert!(matches!(&document.body, Body::Text(text) if text == "# T\nx\n"));
        assert_eq!(document.line_ending, LineEnding::Crlf);
        assert!(document.editable());
        assert_eq!(document.save_bytes().unwrap(), b"# T\r\nx\r\n");
    }

    #[test]
    fn text_without_extension_is_plain() {
        let path = temp("README", b"hello");
        assert_eq!(probe(&path, &[]).unwrap().kind, Kind::Plain { wrap: true });
    }

    #[test]
    fn binary_without_extension_is_refused() {
        let path = temp("blob", &[1, 0, 2]);
        assert!(matches!(probe(&path, &[]), Err(LoadError::Binary)));
    }

    #[test]
    fn other_extensions_are_unsupported() {
        let path = temp("a.exe", b"MZ");
        assert!(matches!(probe(&path, &[]), Err(LoadError::Unsupported)));
    }

    #[test]
    fn missing_files_are_io_errors() {
        let path = std::env::temp_dir()
            .join("tex-reader-missing")
            .join("x.txt");
        assert!(matches!(probe(&path, &[]), Err(LoadError::Io(_))));
    }

    #[test]
    fn medium_files_are_text_but_not_editable() {
        let text = "x".repeat(EDIT_LIMIT as usize + 1);
        let document = open(&temp("m.md", text.as_bytes()));
        assert!(matches!(document.body, Body::Text(_)));
        assert!(!document.editable());
    }

    #[test]
    fn large_files_are_read_only_lines() {
        let text: String = (0..300_000).map(|i| format!("linha {i}\n")).collect();
        assert!(text.len() as u64 > TEXT_LIMIT);
        let path = temp("big.log", text.as_bytes());
        let probed = probe(&path, &[]).unwrap();
        assert!(probed.large());
        let preview = probed.preview_lines();
        assert_eq!(preview[0], "linha 0");
        assert!(preview.last().unwrap().starts_with("linha "));
        assert!(preview.len() < 300_000);
        let document = probed.load(&AtomicBool::new(false)).unwrap().unwrap();
        assert!(!document.editable());
        assert!(document.save_bytes().is_none());
        let Body::Lines(index) = &document.body else {
            panic!("expected lines")
        };
        assert_eq!(index.len(), 300_000);
        let mut file = std::fs::File::open(&path).unwrap();
        assert_eq!(
            index.read_lines(&mut file, 299_999..300_000).unwrap(),
            ["linha 299999"]
        );
    }

    #[test]
    fn edits_mark_the_document_dirty_until_saved() {
        let path = temp("a.txt", b"um");
        let mut document = open(&path);
        document.set_text("um".into());
        assert!(!document.dirty);
        document.set_text("dois".into());
        assert!(document.dirty);
        std::fs::write(&path, b"dois").unwrap();
        let stamp = Stamp::read(&path).unwrap();
        assert!(document.changed(stamp));
        document.saved(stamp);
        assert!(!document.dirty);
        assert!(!document.changed(stamp));
    }
}
