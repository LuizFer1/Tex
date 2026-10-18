use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::encoding::{
    Encoding, LineEnding, decode, decode_body, detect, detect_line_ending, encode, fits,
};
use crate::kind::{Kind, Sniff, sniff};
use crate::lines::LineIndex;

pub const TEXT_LIMIT: u64 = 2 * 1024 * 1024;
/// The largest text that enters edit mode; it does not limit saving, so an edit may grow past it.
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

    /// Opens the file once and reads it whole (up to `TEXT_LIMIT`) or indexes its lines, whatever its size was at the probe; `None` when cancelled.
    pub fn load(self, cancel: &AtomicBool) -> Result<Option<Document>, LoadError> {
        let mut file = File::open(&self.path)?;
        let stamp = Stamp::of(&file.metadata()?);
        let mut bytes = Vec::new();
        if stamp.len <= TEXT_LIMIT {
            (&file).take(TEXT_LIMIT + 1).read_to_end(&mut bytes)?;
        }
        if bytes.len() as u64 > TEXT_LIMIT || stamp.len > TEXT_LIMIT {
            file.seek(SeekFrom::Start(0))?;
            let Some(index) = LineIndex::build(&mut file, self.encoding, cancel)? else {
                return Ok(None);
            };
            let head = &self.head[self.encoding.bom_len().min(self.head.len())..];
            let line_ending = detect_line_ending(&decode_body(head, self.encoding));
            return Ok(Some(Document {
                path: self.path,
                kind: self.kind,
                encoding: self.encoding,
                line_ending,
                stamp,
                body: Body::Lines(index),
                dirty: false,
                lossless: true,
            }));
        }
        let decoded = decode(&bytes);
        Ok(Some(Document {
            path: self.path,
            kind: self.kind,
            encoding: decoded.encoding,
            line_ending: decoded.line_ending,
            stamp,
            body: Body::Text(decoded.text),
            dirty: false,
            lossless: decoded.lossless,
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
    pub lossless: bool,
}

impl Document {
    /// Whether the text may enter edit mode (see `EDIT_LIMIT`).
    pub fn editable(&self) -> bool {
        matches!(&self.body, Body::Text(text) if text.len() as u64 <= EDIT_LIMIT)
    }

    pub fn changed(&self, current: Stamp) -> bool {
        self.stamp != current
    }

    /// Replaces the text and marks the document dirty; returns whether the text changed.
    pub fn set_text(&mut self, text: String) -> bool {
        if let Body::Text(current) = &mut self.body
            && *current != text
        {
            *current = text;
            self.dirty = true;
            return true;
        }
        false
    }

    pub fn save_bytes(&self) -> Option<Vec<u8>> {
        match &self.body {
            Body::Text(text) => Some(encode(text, self.encoding, self.line_ending)),
            Body::Lines(_) => None,
        }
    }

    /// Whether saving keeps every character and byte read; the UI must warn before saving when it is false.
    pub fn fits(&self) -> bool {
        match &self.body {
            Body::Text(text) => self.lossless && fits(text, self.encoding),
            Body::Lines(_) => true,
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

    struct Temp {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fresh_dir() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tex-reader-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temp(name: &str, bytes: &[u8]) -> Temp {
        let dir = fresh_dir();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        Temp { dir, path }
    }

    fn open(path: &Path) -> Document {
        probe(path, &[])
            .unwrap()
            .load(&AtomicBool::new(false))
            .unwrap()
            .unwrap()
    }

    fn many_lines() -> String {
        let text: String = (0..300_000).map(|i| format!("linha {i}\n")).collect();
        assert!(text.len() as u64 > TEXT_LIMIT);
        text
    }

    #[test]
    fn small_markdown_loads_as_editable_text() {
        let file = temp("n.md", b"# T\r\nx\r\n");
        let path = &file.path;
        let probed = probe(path, &[]).unwrap();
        assert_eq!(probed.kind, Kind::Markdown);
        assert!(!probed.large());
        let document = open(path);
        assert!(matches!(&document.body, Body::Text(text) if text == "# T\nx\n"));
        assert_eq!(document.line_ending, LineEnding::Crlf);
        assert!(document.editable());
        assert_eq!(document.save_bytes().unwrap(), b"# T\r\nx\r\n");
    }

    #[test]
    fn text_without_extension_is_plain() {
        let file = temp("README", b"hello");
        assert_eq!(
            probe(&file.path, &[]).unwrap().kind,
            Kind::Plain { wrap: true }
        );
    }

    #[test]
    fn binary_without_extension_is_refused() {
        let file = temp("blob", &[1, 0, 2]);
        assert!(matches!(probe(&file.path, &[]), Err(LoadError::Binary)));
    }

    #[test]
    fn other_extensions_are_unsupported() {
        let file = temp("a.exe", b"MZ");
        assert!(matches!(
            probe(&file.path, &[]),
            Err(LoadError::Unsupported)
        ));
    }

    #[test]
    fn missing_files_are_io_errors() {
        let dir = fresh_dir();
        let guard = Temp {
            path: dir.join("x.txt"),
            dir,
        };
        assert!(matches!(probe(&guard.path, &[]), Err(LoadError::Io(_))));
    }

    #[test]
    fn medium_files_are_text_but_not_editable() {
        let text = "x".repeat(EDIT_LIMIT as usize + 1);
        let file = temp("m.md", text.as_bytes());
        let document = open(&file.path);
        assert!(matches!(document.body, Body::Text(_)));
        assert!(!document.editable());
    }

    #[test]
    fn large_files_are_read_only_lines() {
        let text = many_lines();
        let file = temp("big.log", text.as_bytes());
        let path = &file.path;
        let probed = probe(path, &[]).unwrap();
        assert!(probed.large());
        let preview = probed.preview_lines();
        assert_eq!(preview[0], "linha 0");
        assert!(preview.last().unwrap().starts_with("linha "));
        assert!(preview.len() < 300_000);
        let mut document = probed.load(&AtomicBool::new(false)).unwrap().unwrap();
        assert!(!document.editable());
        assert!(document.save_bytes().is_none());
        assert!(!document.set_text("x".into()));
        assert!(!document.dirty);
        let Body::Lines(index) = &document.body else {
            panic!("expected lines")
        };
        assert_eq!(index.len(), 300_000);
        let mut handle = std::fs::File::open(path).unwrap();
        assert_eq!(
            index.read_lines(&mut handle, 299_999..300_000).unwrap(),
            ["linha 299999"]
        );
    }

    #[test]
    fn edits_mark_the_document_dirty_until_saved() {
        let file = temp("a.txt", b"um");
        let path = &file.path;
        let mut document = open(path);
        assert!(!document.set_text("um".into()));
        assert!(!document.dirty);
        assert!(document.set_text("dois".into()));
        assert!(document.dirty);
        std::fs::write(path, b"dois").unwrap();
        let stamp = Stamp::read(path).unwrap();
        assert!(document.changed(stamp));
        document.saved(stamp);
        assert!(!document.dirty);
        assert!(!document.changed(stamp));
    }

    #[test]
    fn cancelled_loads_of_large_files_give_none() {
        let file = temp("big.log", many_lines().as_bytes());
        let probed = probe(&file.path, &[]).unwrap();
        assert!(probed.load(&AtomicBool::new(true)).unwrap().is_none());
    }

    #[test]
    fn the_edit_limit_is_inclusive() {
        let at = temp("e.txt", &vec![b'x'; EDIT_LIMIT as usize]);
        assert!(open(&at.path).editable());
        let over = temp("o.txt", &vec![b'x'; EDIT_LIMIT as usize + 1]);
        assert!(!open(&over.path).editable());
    }

    #[test]
    fn an_edit_that_grows_past_the_edit_limit_still_saves() {
        let file = temp("g.txt", b"x");
        let mut document = open(&file.path);
        let grown = "y".repeat(EDIT_LIMIT as usize + 1);
        assert!(document.set_text(grown.clone()));
        assert!(!document.editable());
        assert!(document.dirty);
        assert_eq!(document.save_bytes().unwrap(), grown.as_bytes());
    }

    #[test]
    fn a_file_of_exactly_the_text_limit_is_text() {
        let file = temp("t.txt", &vec![b'x'; TEXT_LIMIT as usize]);
        let probed = probe(&file.path, &[]).unwrap();
        assert!(!probed.large());
        let document = probed.load(&AtomicBool::new(false)).unwrap().unwrap();
        assert!(matches!(document.body, Body::Text(_)));
        assert_eq!(document.stamp.len, TEXT_LIMIT);
    }

    #[test]
    fn a_file_that_grows_after_the_probe_is_indexed() {
        let file = temp("grow.log", b"um\n");
        let probed = probe(&file.path, &[]).unwrap();
        assert!(!probed.large());
        let text = many_lines();
        std::fs::write(&file.path, text.as_bytes()).unwrap();
        let document = probed.load(&AtomicBool::new(false)).unwrap().unwrap();
        assert!(matches!(&document.body, Body::Lines(index) if index.len() == 300_000));
        assert_eq!(document.stamp.len, text.len() as u64);
        assert!(!document.editable());
    }

    #[test]
    fn utf16_with_crlf_round_trips() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "olá\r\nmundo".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        let file = temp("u.txt", &bytes);
        let probed = probe(&file.path, &[]).unwrap();
        assert_eq!(probed.preview_lines(), ["olá", "mundo"]);
        let document = open(&file.path);
        assert_eq!(document.encoding, Encoding::Utf16Le);
        assert_eq!(document.line_ending, LineEnding::Crlf);
        assert!(matches!(&document.body, Body::Text(text) if text == "olá\nmundo"));
        assert_eq!(document.save_bytes().unwrap(), bytes);
    }

    #[test]
    fn preview_of_a_short_file_keeps_the_last_line() {
        let file = temp("p.txt", b"a\r\nb");
        let probed = probe(&file.path, &[]).unwrap();
        assert_eq!(probed.preview_lines(), ["a", "b"]);
    }

    #[test]
    fn saving_uses_the_original_line_ending() {
        let file = temp("s.txt", b"a\r\nb\r\n");
        let mut document = open(&file.path);
        document.set_text("x\ny\n".into());
        assert_eq!(document.save_bytes().unwrap(), b"x\r\ny\r\n");
    }

    #[test]
    fn fits_is_false_when_windows_1252_would_lose_characters() {
        let file = temp("w.txt", &[b'c', 0xE9, b'!', b'\n']);
        let mut document = open(&file.path);
        assert_eq!(document.encoding, Encoding::Windows1252);
        assert!(document.fits());
        document.set_text("日本\n".into());
        assert!(!document.fits());
    }

    #[test]
    fn invalid_bytes_after_a_bom_do_not_fit() {
        let file = temp("b.txt", &[0xEF, 0xBB, 0xBF, b'a', 0xFF]);
        let document = open(&file.path);
        assert!(matches!(document.body, Body::Text(_)));
        assert!(!document.fits());
    }

    #[test]
    fn large_files_keep_their_line_ending() {
        let text = many_lines().replace('\n', "\r\n");
        let file = temp("crlf.log", text.as_bytes());
        let document = open(&file.path);
        assert!(matches!(document.body, Body::Lines(_)));
        assert_eq!(document.line_ending, LineEnding::Crlf);
    }
}
