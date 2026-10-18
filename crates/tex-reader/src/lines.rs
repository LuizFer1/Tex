use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::encoding::{Encoding, decode_body};

pub const MAX_LINE_BYTES: usize = 64 * 1024;
const STRIDE: usize = 64;
const CHUNK: usize = 1024 * 1024;

/// Where every `STRIDE`-th line starts, so a large file is read a page at a time.
#[derive(Debug)]
pub struct LineIndex {
    encoding: Encoding,
    checkpoints: Vec<u64>,
    lines: usize,
}

impl LineIndex {
    /// Indexes `reader`; `None` when `cancel` was set before the end.
    pub fn build<R: Read + Seek>(
        reader: &mut R,
        encoding: Encoding,
        cancel: &AtomicBool,
    ) -> io::Result<Option<LineIndex>> {
        Self::build_in_chunks(reader, encoding, cancel, CHUNK)
    }

    fn build_in_chunks<R: Read + Seek>(
        reader: &mut R,
        encoding: Encoding,
        cancel: &AtomicBool,
        chunk: usize,
    ) -> io::Result<Option<LineIndex>> {
        let end = reader.seek(SeekFrom::End(0))?;
        let start = (encoding.bom_len() as u64).min(end);
        reader.seek(SeekFrom::Start(start))?;
        let unit = encoding.unit_len();
        let newline = encoding.newline();
        let mut checkpoints = vec![start];
        let mut lines = 1;
        let mut last_start = start;
        let mut buffer = vec![0; chunk.max(2)];
        let mut position = start;
        let mut carry = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let read = reader.read(&mut buffer[carry..])?;
            if read == 0 {
                break;
            }
            let filled = carry + read;
            let usable = filled - filled % unit;
            for (at, code) in buffer[..usable].chunks_exact(unit).enumerate() {
                if code == newline {
                    let next = position + ((at + 1) * unit) as u64;
                    if lines % STRIDE == 0 {
                        checkpoints.push(next);
                    }
                    last_start = next;
                    lines += 1;
                }
            }
            buffer.copy_within(usable..filled, 0);
            carry = filled - usable;
            position += usable as u64;
        }
        if last_start == end && lines > 1 {
            lines -= 1;
            if lines % STRIDE == 0 {
                checkpoints.pop();
            }
        }
        Ok(Some(LineIndex {
            encoding,
            checkpoints,
            lines,
        }))
    }

    pub fn len(&self) -> usize {
        self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The lines in `range` (clamped to the file), each cut at `MAX_LINE_BYTES` and then ended with `…`.
    pub fn read_lines<R: Read + Seek>(
        &self,
        reader: &mut R,
        range: Range<usize>,
    ) -> io::Result<Vec<String>> {
        let range = range.start.min(self.lines)..range.end.min(self.lines);
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let checkpoint = range.start / STRIDE;
        reader.seek(SeekFrom::Start(self.checkpoints[checkpoint]))?;
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        for _ in checkpoint * STRIDE..range.start {
            read_line(&mut reader, self.encoding, &mut bytes)?;
        }
        let mut out = Vec::with_capacity(range.len());
        for _ in range {
            let cut = read_line(&mut reader, self.encoding, &mut bytes)?.unwrap_or(false);
            let mut line = decode_body(&bytes, self.encoding);
            if cut {
                line.push('…');
            }
            out.push(line);
        }
        Ok(out)
    }
}

/// Reads one line, without its break, into `out` (at most `MAX_LINE_BYTES`, never ending in half a character): `Some(cut)` or `None` at the end of the file.
fn read_line(
    reader: &mut impl BufRead,
    encoding: Encoding,
    out: &mut Vec<u8>,
) -> io::Result<Option<bool>> {
    out.clear();
    let unit = encoding.unit_len();
    let newline = encoding.newline();
    let mut dropped = 0;
    let mut first_dropped = [0u8; 2];
    let mut pending = None;
    let mut any = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let mut start = 0;
        let mut found = false;
        let mut end = buffer.len();
        if let Some(first) = pending.take() {
            let pair = [first, buffer[0]];
            any = true;
            start = 1;
            if pair == newline {
                found = true;
                end = 1;
            } else {
                keep(out, &pair, &mut dropped, &mut first_dropped);
            }
        }
        let consumed = if found {
            end
        } else if unit == 1 {
            let at = buffer[start..].iter().position(|&byte| byte == newline[0]);
            let stop = at.map_or(buffer.len(), |at| start + at);
            keep(out, &buffer[start..stop], &mut dropped, &mut first_dropped);
            any |= stop > start || at.is_some();
            found = at.is_some();
            stop + usize::from(found)
        } else {
            let (pairs, rest) = buffer[start..].as_chunks::<2>();
            let at = pairs.iter().position(|pair| pair == newline);
            let stop = start + 2 * at.unwrap_or(pairs.len());
            keep(out, &buffer[start..stop], &mut dropped, &mut first_dropped);
            any |= stop > start || at.is_some();
            found = at.is_some();
            if !found {
                pending = rest.first().copied();
            }
            if found { stop + 2 } else { buffer.len() }
        };
        reader.consume(consumed);
        if found {
            break;
        }
    }
    if !any {
        return Ok(None);
    }
    let cut = dropped > 0 && !(dropped == unit && first_dropped[..unit] == *encoding.carriage());
    if cut {
        trim_partial(out, encoding);
    } else if dropped == 0 && out.ends_with(encoding.carriage()) {
        out.truncate(out.len() - unit);
    }
    Ok(Some(cut))
}

/// Appends what still fits under `MAX_LINE_BYTES` and counts the rest, remembering its first unit.
fn keep(out: &mut Vec<u8>, bytes: &[u8], dropped: &mut usize, first_dropped: &mut [u8; 2]) {
    let room = MAX_LINE_BYTES.saturating_sub(out.len());
    let taken = bytes.len().min(room);
    out.extend_from_slice(&bytes[..taken]);
    if taken < bytes.len() {
        if *dropped == 0 {
            let first = &bytes[taken..];
            first_dropped[..first.len().min(2)].copy_from_slice(&first[..first.len().min(2)]);
        }
        *dropped += bytes.len() - taken;
    }
}

/// Removes a character the cut left incomplete: a UTF-8 tail or a UTF-16 high surrogate.
fn trim_partial(out: &mut Vec<u8>, encoding: Encoding) {
    match encoding {
        Encoding::Utf8 { .. } => {
            if let Err(error) = std::str::from_utf8(out)
                && error.error_len().is_none()
            {
                out.truncate(error.valid_up_to());
            }
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            if let Some(&[a, b]) = out.last_chunk::<2>() {
                let unit = if encoding == Encoding::Utf16Le {
                    u16::from_le_bytes([a, b])
                } else {
                    u16::from_be_bytes([a, b])
                };
                if (0xD800..0xDC00).contains(&unit) {
                    out.truncate(out.len() - 2);
                }
            }
        }
        Encoding::Windows1252 => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const UTF8: Encoding = Encoding::Utf8 { bom: false };

    fn index(bytes: &[u8], encoding: Encoding, chunk: usize) -> (LineIndex, Cursor<Vec<u8>>) {
        let mut cursor = Cursor::new(bytes.to_vec());
        let cancel = AtomicBool::new(false);
        let index = LineIndex::build_in_chunks(&mut cursor, encoding, &cancel, chunk)
            .unwrap()
            .unwrap();
        (index, cursor)
    }

    fn all(bytes: &[u8], encoding: Encoding, chunk: usize) -> Vec<String> {
        let (index, mut cursor) = index(bytes, encoding, chunk);
        index.read_lines(&mut cursor, 0..index.len()).unwrap()
    }

    #[test]
    fn splits_lf_and_crlf() {
        assert_eq!(all(b"a\nb\r\nc", UTF8, 3), ["a", "b", "c"]);
    }

    #[test]
    fn a_final_break_ends_the_last_line() {
        assert_eq!(all(b"a\nb\n", UTF8, CHUNK), ["a", "b"]);
    }

    #[test]
    fn an_empty_file_has_one_empty_line() {
        assert_eq!(all(b"", UTF8, CHUNK), [""]);
    }

    #[test]
    fn the_bom_is_not_text() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(b"a\nb");
        assert_eq!(all(&bytes, Encoding::Utf8 { bom: true }, 2), ["a", "b"]);
    }

    #[test]
    fn utf16_survives_odd_chunks() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "x\r\nyé\n".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(all(&bytes, Encoding::Utf16Le, 3), ["x", "yé"]);
    }

    #[test]
    fn ranges_start_from_checkpoints() {
        let text: String = (0..200).map(|i| format!("line{i}\n")).collect();
        let (index, mut cursor) = index(text.as_bytes(), UTF8, 7);
        assert_eq!(index.len(), 200);
        assert_eq!(
            index.read_lines(&mut cursor, 130..133).unwrap(),
            ["line130", "line131", "line132"]
        );
        assert_eq!(
            index.read_lines(&mut cursor, 198..250).unwrap(),
            ["line198", "line199"]
        );
        assert!(index.read_lines(&mut cursor, 300..310).unwrap().is_empty());
    }

    #[test]
    fn long_lines_are_cut() {
        let text = "x".repeat(MAX_LINE_BYTES + 10) + "\nok";
        let lines = all(text.as_bytes(), UTF8, CHUNK);
        assert_eq!(lines[0].chars().count(), MAX_LINE_BYTES + 1);
        assert!(lines[0].ends_with('…'));
        assert_eq!(lines[1], "ok");
    }

    fn numbered(count: usize, trailing_newline: bool) -> String {
        let mut text = (0..count)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        if trailing_newline {
            text.push('\n');
        }
        text
    }

    #[test]
    fn checkpoint_boundaries_keep_every_line() {
        for count in [64, 65, 128] {
            for trailing in [false, true] {
                let text = numbered(count, trailing);
                let (index, mut cursor) = index(text.as_bytes(), UTF8, 5);
                assert_eq!(index.len(), count, "{count} {trailing}");
                let last = count - 3;
                let expected: Vec<String> = (last..count).map(|i| format!("l{i}")).collect();
                assert_eq!(
                    index.read_lines(&mut cursor, last..count).unwrap(),
                    expected
                );
                let expected: Vec<String> = (60..70.min(count)).map(|i| format!("l{i}")).collect();
                assert_eq!(index.read_lines(&mut cursor, 60..70).unwrap(), expected);
            }
        }
    }

    #[test]
    fn a_line_of_exactly_the_limit_with_crlf_is_not_cut() {
        let text = "x".repeat(MAX_LINE_BYTES) + "\r\nok";
        assert_eq!(
            all(text.as_bytes(), UTF8, CHUNK),
            ["x".repeat(MAX_LINE_BYTES), "ok".to_string()]
        );
    }

    #[test]
    fn a_cut_inside_a_character_drops_it() {
        let text = "x".repeat(MAX_LINE_BYTES - 1) + "éyy\nok";
        let lines = all(text.as_bytes(), UTF8, CHUNK);
        assert_eq!(lines[0], "x".repeat(MAX_LINE_BYTES - 1) + "…");
        assert!(!lines[0].contains('\u{FFFD}'));
        assert_eq!(lines[1], "ok");
    }

    fn utf16_be(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend(unit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn utf16_big_endian_survives_small_chunks() {
        assert_eq!(
            all(&utf16_be("x\r\nyé\nz"), Encoding::Utf16Be, 3),
            ["x", "yé", "z"]
        );
    }

    #[test]
    fn long_utf16_lines_are_cut_between_characters() {
        let half = MAX_LINE_BYTES / 2;
        let text = "x".repeat(half - 1) + "😀yy\nok";
        let lines = all(&utf16_be(&text), Encoding::Utf16Be, 3);
        assert_eq!(lines[0], "x".repeat(half - 1) + "…");
        assert_eq!(lines[1], "ok");
        let text = "x".repeat(half + 10) + "\nok";
        let lines = all(&utf16_be(&text), Encoding::Utf16Be, CHUNK);
        assert_eq!(lines[0].chars().count(), half + 1);
        assert_eq!(lines[1], "ok");
    }

    #[test]
    fn skipping_a_huge_line_reaches_the_next() {
        let mut text = vec![b'x'; 20 * 1024 * 1024];
        text.extend(b"\nok");
        let (index, mut cursor) = index(&text, UTF8, CHUNK);
        assert_eq!(index.read_lines(&mut cursor, 1..2).unwrap(), ["ok"]);
    }

    #[test]
    fn cancel_stops_the_build() {
        let cancel = AtomicBool::new(true);
        let mut cursor = Cursor::new(b"a\n".to_vec());
        assert!(
            LineIndex::build(&mut cursor, UTF8, &cancel)
                .unwrap()
                .is_none()
        );
    }
}
