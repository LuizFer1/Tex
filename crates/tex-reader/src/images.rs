use std::path::{Path, PathBuf};

pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
#[allow(dead_code)]
pub const MAX_PIXELS: u64 = 64 * 1_000_000;
#[allow(dead_code)]
pub const MAX_FRAMES: usize = 300;
#[allow(dead_code)]
pub const MAX_RGBA_BYTES: u64 = 128 * 1024 * 1024;
#[allow(dead_code)]
const MIN_DELAY_MS: u32 = 20;
#[allow(dead_code)]
const DEFAULT_DELAY_MS: u32 = 100;

/// Why an image is not shown; the codes are the ones `ui/reader.slint` turns into sentences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Unsupported = 1,
    Unreadable = 2,
    Network = 3,
    TooLarge = 4,
    Corrupt = 5,
    Insecure = 6,
    RemoteOff = 7,
    Budget = 8,
}

impl Reason {
    pub fn code(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Local(PathBuf),
    Remote(String),
    Unsupported(Reason),
}

/// Where the image `src` of a Markdown file in `md_dir` lives.
pub fn resolve(src: &str, md_dir: &Path) -> Source {
    let src = src.trim();
    if src.is_empty() || has_prefix(src, "data:") || is_svg(src) {
        return Source::Unsupported(Reason::Unsupported);
    }
    if has_prefix(src, "https://") {
        return Source::Remote(src.to_owned());
    }
    if has_prefix(src, "http://") {
        return Source::Unsupported(Reason::Insecure);
    }
    if has_prefix(src, "file://") {
        return file_uri(&src[7..]);
    }
    let decoded = percent_decode(src);
    let path = PathBuf::from(&decoded);
    if path.is_absolute() || has_drive(&decoded) {
        Source::Local(path)
    } else {
        Source::Local(md_dir.join(decoded))
    }
}

fn has_prefix(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_svg(src: &str) -> bool {
    let end = src.find(['?', '#']).unwrap_or(src.len());
    src[..end].to_ascii_lowercase().ends_with(".svg")
}

fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn file_uri(rest: &str) -> Source {
    let decoded = percent_decode(rest);
    if let Some(path) = decoded.strip_prefix('/') {
        if has_drive(path) {
            return Source::Local(PathBuf::from(path));
        }
        return Source::Local(PathBuf::from(decoded));
    }
    let (host, path) = decoded.split_once('/').unwrap_or((decoded.as_str(), ""));
    if host.is_empty() {
        return Source::Unsupported(Reason::Unsupported);
    }
    Source::Local(PathBuf::from(format!(
        "\\\\{host}\\{}",
        path.replace('/', "\\")
    )))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2]))
        {
            out.push(high * 16 + low);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dir() -> &'static Path {
        Path::new("docs")
    }

    #[test]
    fn relative_paths_join_the_markdown_folder() {
        assert_eq!(
            resolve("img/logo.png", dir()),
            Source::Local(Path::new("docs").join("img/logo.png"))
        );
        assert_eq!(
            resolve("my%20pic.png", dir()),
            Source::Local(Path::new("docs").join("my pic.png"))
        );
    }

    #[test]
    fn remote_sources_need_https() {
        assert_eq!(
            resolve("https://example.com/a.png?x=1", dir()),
            Source::Remote("https://example.com/a.png?x=1".into())
        );
        assert_eq!(
            resolve("HTTPS://example.com/a.png", dir()),
            Source::Remote("HTTPS://example.com/a.png".into())
        );
        assert_eq!(
            resolve("http://example.com/a.png", dir()),
            Source::Unsupported(Reason::Insecure)
        );
    }

    #[test]
    fn data_uris_svg_and_empty_sources_are_unsupported() {
        for src in [
            "data:image/png;base64,AAAA",
            "logo.svg",
            "logo.SVG",
            "https://x.org/a.svg?v=2",
            "",
            "  ",
        ] {
            assert_eq!(
                resolve(src, dir()),
                Source::Unsupported(Reason::Unsupported),
                "{src}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn file_uris_and_drive_paths() {
        assert_eq!(
            resolve("file:///C:/a%20b/x.png", dir()),
            Source::Local(std::path::PathBuf::from("C:/a b/x.png"))
        );
        assert_eq!(
            resolve("file://server/share/x.png", dir()),
            Source::Local(std::path::PathBuf::from("\\\\server\\share\\x.png"))
        );
        assert_eq!(
            resolve("C:\\fotos\\x.png", dir()),
            Source::Local(std::path::PathBuf::from("C:\\fotos\\x.png"))
        );
    }
}
