use std::path::Path;

use crate::encoding::has_bom;

pub const CORE_EXTENSIONS: [&str; 15] = [
    "md", "txt", "log", "json", "jsonl", "csv", "tsv", "toml", "yaml", "yml", "ini", "cfg", "conf",
    "env", "xml",
];
const NO_WRAP: [&str; 4] = ["log", "csv", "tsv", "jsonl"];
pub const SNIFF_LEN: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Markdown,
    Plain { wrap: bool },
    Viewer(String),
    Unknown,
}

impl Kind {
    /// How the reader opens `path`, or `None` when it cannot read it. `viewer_exts` are lowercase.
    pub fn from_path(path: &Path, viewer_exts: &[String]) -> Option<Kind> {
        let Some(ext) = path.extension() else {
            return Some(Kind::Unknown);
        };
        let ext = ext.to_str()?.to_ascii_lowercase();
        if viewer_exts.contains(&ext) {
            return Some(Kind::Viewer(ext));
        }
        if ext == "md" {
            return Some(Kind::Markdown);
        }
        CORE_EXTENSIONS
            .contains(&ext.as_str())
            .then(|| Kind::Plain {
                wrap: !NO_WRAP.contains(&ext.as_str()),
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sniff {
    Text,
    Binary,
}

/// Looks at the first `SNIFF_LEN` bytes: a NUL byte without a BOM means binary.
pub fn sniff(head: &[u8]) -> Sniff {
    let head = &head[..head.len().min(SNIFF_LEN)];
    if has_bom(head) || !head.contains(&0) {
        Sniff::Text
    } else {
        Sniff::Binary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn kind(name: &str) -> Option<Kind> {
        Kind::from_path(Path::new(name), &["srt".to_string(), "csv".to_string()])
    }

    #[test]
    fn markdown_ignores_case() {
        assert_eq!(kind("notas.MD"), Some(Kind::Markdown));
    }

    #[test]
    fn logs_do_not_wrap() {
        assert_eq!(kind("app.log"), Some(Kind::Plain { wrap: false }));
        assert_eq!(kind("dados.tsv"), Some(Kind::Plain { wrap: false }));
        assert_eq!(kind("eventos.jsonl"), Some(Kind::Plain { wrap: false }));
        assert_eq!(kind("a.txt"), Some(Kind::Plain { wrap: true }));
        assert_eq!(kind("Cargo.toml"), Some(Kind::Plain { wrap: true }));
    }

    #[test]
    fn no_extension_is_decided_by_content() {
        assert_eq!(kind("README"), Some(Kind::Unknown));
        assert_eq!(kind(".gitignore"), Some(Kind::Unknown));
    }

    #[test]
    fn viewers_win_over_the_core() {
        assert_eq!(kind("filme.srt"), Some(Kind::Viewer("srt".into())));
        assert_eq!(kind("dados.CSV"), Some(Kind::Viewer("csv".into())));
    }

    #[test]
    fn other_extensions_are_not_readable() {
        assert_eq!(kind("setup.exe"), None);
        assert_eq!(kind("foto.png"), None);
        assert_eq!(kind("arquivo."), None);
    }

    #[test]
    fn sniff_finds_binaries() {
        assert_eq!(sniff(b"hello\n"), Sniff::Text);
        assert_eq!(sniff(&[b'M', b'Z', 0, 0]), Sniff::Binary);
        assert_eq!(sniff(&[0xFF, 0xFE, b'a', 0]), Sniff::Text);
        let mut late = vec![b'a'; SNIFF_LEN];
        late.push(0);
        assert_eq!(sniff(&late), Sniff::Text);
    }
}
