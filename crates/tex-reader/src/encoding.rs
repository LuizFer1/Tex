use std::borrow::Cow;

pub(crate) const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
pub(crate) const UTF16LE_BOM: [u8; 2] = [0xFF, 0xFE];
pub(crate) const UTF16BE_BOM: [u8; 2] = [0xFE, 0xFF];

const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8 { bom: bool },
    Utf16Le,
    Utf16Be,
    Windows1252,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Crlf,
    Lf,
}

#[derive(Debug)]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

impl Encoding {
    pub fn bom_len(self) -> usize {
        match self {
            Encoding::Utf8 { bom: true } => 3,
            Encoding::Utf16Le | Encoding::Utf16Be => 2,
            Encoding::Utf8 { bom: false } | Encoding::Windows1252 => 0,
        }
    }

    pub(crate) fn unit_len(self) -> usize {
        match self {
            Encoding::Utf16Le | Encoding::Utf16Be => 2,
            _ => 1,
        }
    }

    pub(crate) fn newline(self) -> &'static [u8] {
        match self {
            Encoding::Utf16Le => &[0x0A, 0],
            Encoding::Utf16Be => &[0, 0x0A],
            _ => b"\n",
        }
    }

    pub(crate) fn carriage(self) -> &'static [u8] {
        match self {
            Encoding::Utf16Le => &[0x0D, 0],
            Encoding::Utf16Be => &[0, 0x0D],
            _ => b"\r",
        }
    }
}

impl LineEnding {
    pub fn native() -> LineEnding {
        if cfg!(windows) {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

pub(crate) fn has_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&UTF8_BOM)
        || bytes.starts_with(&UTF16LE_BOM)
        || bytes.starts_with(&UTF16BE_BOM)
}

/// The BOM decides; without one, UTF-8 (a character cut at the end still counts) or else Windows-1252.
pub fn detect(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&UTF8_BOM) {
        return Encoding::Utf8 { bom: true };
    }
    if bytes.starts_with(&UTF16LE_BOM) {
        return Encoding::Utf16Le;
    }
    if bytes.starts_with(&UTF16BE_BOM) {
        return Encoding::Utf16Be;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => Encoding::Utf8 { bom: false },
        Err(error) if error.error_len().is_none() => Encoding::Utf8 { bom: false },
        Err(_) => Encoding::Windows1252,
    }
}

/// Decodes a whole file: text with `\n` breaks, plus what is needed to write it back the same way.
pub fn decode(bytes: &[u8]) -> Decoded {
    let encoding = detect(bytes);
    let raw = decode_body(&bytes[encoding.bom_len()..], encoding);
    let line_ending = detect_line_ending(&raw);
    let text = if raw.contains('\r') {
        raw.replace("\r\n", "\n")
    } else {
        raw
    };
    Decoded {
        text,
        encoding,
        line_ending,
    }
}

pub(crate) fn decode_body(body: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 { .. } => String::from_utf8_lossy(body).into_owned(),
        Encoding::Utf16Le => decode_utf16(body, u16::from_le_bytes),
        Encoding::Utf16Be => decode_utf16(body, u16::from_be_bytes),
        Encoding::Windows1252 => body.iter().map(|&byte| cp1252_char(byte)).collect(),
    }
}

fn decode_utf16(body: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = body.as_chunks::<2>().0.iter().map(|pair| unit(*pair));
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn detect_line_ending(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf) = (0usize, 0usize);
    for (at, _) in text.match_indices('\n').take(1000) {
        if at > 0 && bytes[at - 1] == b'\r' {
            crlf += 1;
        } else {
            lf += 1;
        }
    }
    match (crlf, lf) {
        (0, 0) => LineEnding::native(),
        (crlf, lf) if crlf >= lf => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

/// Encodes `text` (with `\n` breaks) the way the file was: same encoding, BOM and line ending.
pub fn encode(text: &str, encoding: Encoding, line_ending: LineEnding) -> Vec<u8> {
    let text: Cow<str> = match line_ending {
        LineEnding::Crlf => Cow::Owned(text.replace('\n', "\r\n")),
        LineEnding::Lf => Cow::Borrowed(text),
    };
    let mut out = Vec::with_capacity(text.len() + 3);
    match encoding {
        Encoding::Utf8 { bom } => {
            if bom {
                out.extend_from_slice(&UTF8_BOM);
            }
            out.extend_from_slice(text.as_bytes());
        }
        Encoding::Utf16Le => {
            out.extend_from_slice(&UTF16LE_BOM);
            text.encode_utf16()
                .for_each(|unit| out.extend(unit.to_le_bytes()));
        }
        Encoding::Utf16Be => {
            out.extend_from_slice(&UTF16BE_BOM);
            text.encode_utf16()
                .for_each(|unit| out.extend(unit.to_be_bytes()));
        }
        Encoding::Windows1252 => out.extend(text.chars().map(cp1252_byte)),
    }
    out
}

/// Whether `encode` keeps every character of `text` (only Windows-1252 can lose some).
pub fn fits(text: &str, encoding: Encoding) -> bool {
    encoding != Encoding::Windows1252 || text.chars().all(|c| c == '?' || cp1252_byte(c) != b'?')
}

fn cp1252_char(byte: u8) -> char {
    match byte {
        0x80..=0x9F => CP1252_HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

fn cp1252_byte(c: char) -> u8 {
    match u32::from(c) {
        0..=0x7F | 0xA0..=0xFF => c as u8,
        _ => CP1252_HIGH
            .iter()
            .position(|&high| high == c)
            .map_or(b'?', |at| 0x80 + at as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, little: bool) -> Vec<u8> {
        let mut bytes = if little {
            UTF16LE_BOM.to_vec()
        } else {
            UTF16BE_BOM.to_vec()
        };
        for unit in text.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        bytes
    }

    fn round_trip(bytes: &[u8]) -> Decoded {
        let decoded = decode(bytes);
        assert_eq!(
            encode(&decoded.text, decoded.encoding, decoded.line_ending),
            bytes
        );
        decoded
    }

    #[test]
    fn utf8_without_bom() {
        let decoded = round_trip("olá\nmundo\n".as_bytes());
        assert_eq!(decoded.text, "olá\nmundo\n");
        assert_eq!(decoded.encoding, Encoding::Utf8 { bom: false });
        assert_eq!(decoded.line_ending, LineEnding::Lf);
    }

    #[test]
    fn utf8_with_bom_and_crlf() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend(b"a\r\nb\r\n");
        let decoded = round_trip(&bytes);
        assert_eq!(decoded.text, "a\nb\n");
        assert_eq!(decoded.encoding, Encoding::Utf8 { bom: true });
        assert_eq!(decoded.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn utf16_in_both_orders() {
        for (bytes, encoding) in [
            (utf16("é\r\nz", true), Encoding::Utf16Le),
            (utf16("é\r\nz", false), Encoding::Utf16Be),
        ] {
            let decoded = round_trip(&bytes);
            assert_eq!(decoded.text, "é\nz");
            assert_eq!(decoded.encoding, encoding);
            assert_eq!(decoded.line_ending, LineEnding::Crlf);
        }
    }

    #[test]
    fn invalid_utf8_is_windows_1252() {
        let decoded = round_trip(&[b'c', 0xE9, b'!', 0x80, 0x81, b'\n']);
        assert_eq!(decoded.text, "cé!€\u{81}\n");
        assert_eq!(decoded.encoding, Encoding::Windows1252);
    }

    #[test]
    fn cut_utf8_tail_is_still_utf8() {
        assert_eq!(
            decode(&"olá".as_bytes()[..3]).encoding,
            Encoding::Utf8 { bom: false }
        );
    }

    #[test]
    fn the_most_common_line_ending_wins() {
        assert_eq!(decode(b"a\r\nb\r\nc\n").line_ending, LineEnding::Crlf);
        assert_eq!(decode(b"a\nb\nc\r\n").line_ending, LineEnding::Lf);
        assert_eq!(decode(b"abc").line_ending, LineEnding::native());
    }

    #[test]
    fn windows_1252_cannot_hold_everything() {
        assert!(fits("café €", Encoding::Windows1252));
        assert!(fits("?", Encoding::Windows1252));
        assert!(!fits("日本", Encoding::Windows1252));
        assert!(fits("日本", Encoding::Utf8 { bom: false }));
        assert_eq!(encode("日", Encoding::Windows1252, LineEnding::Lf), b"?");
    }
}
