#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

use tex_platform::WorkArea;
use tex_reader::{Block, Encoding, Kind, MAX_LINE_BYTES};

pub const PAGE_LINES: usize = 128;
pub const CACHED_PAGES: usize = 32;
const MAX_WIDTH: f32 = 1100.0;
const MAX_HEIGHT: f32 = 900.0;

/// Logical size of a new reader window: 70% × 80% of the work area, at most 1100 × 900.
pub fn reader_size(area: &WorkArea) -> (f32, f32) {
    let width = area.width as f32 / area.scale;
    let height = area.height as f32 / area.scale;
    (
        (width * 0.7).min(MAX_WIDTH).round(),
        (height * 0.8).min(MAX_HEIGHT).round(),
    )
}

/// Cuts a line the way `LineIndex` does: at `MAX_LINE_BYTES`, on a character boundary, then `…`.
pub fn display_line(line: &str) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line.to_string();
    }
    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// The lines of a text held in memory, without the empty one after a final break.
pub fn display_lines(text: &str) -> Vec<String> {
    text.strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .map(display_line)
        .collect()
}

pub fn longest(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    Hit(&'a str),
    Miss { request: Option<usize> },
}

/// The pages of a large file already read, oldest evicted first; each missing page is requested once.
pub struct PageCache {
    pages: HashMap<usize, Vec<String>>,
    order: VecDeque<usize>,
    requested: HashSet<usize>,
    capacity: usize,
}

impl PageCache {
    pub fn new(capacity: usize) -> PageCache {
        PageCache {
            pages: HashMap::new(),
            order: VecDeque::new(),
            requested: HashSet::new(),
            capacity,
        }
    }

    pub fn get(&mut self, row: usize) -> Lookup<'_> {
        let page = row / PAGE_LINES;
        if let Some(lines) = self.pages.get(&page) {
            return Lookup::Hit(lines.get(row % PAGE_LINES).map_or("", String::as_str));
        }
        Lookup::Miss {
            request: self.requested.insert(page).then_some(page),
        }
    }

    pub fn insert(&mut self, page: usize, lines: Vec<String>) {
        self.requested.remove(&page);
        if self.pages.insert(page, lines).is_none() {
            self.order.push_back(page);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.pages.remove(&oldest);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Web(String),
    Read(PathBuf),
    Reveal(PathBuf),
    Ignore,
}

/// Where a Markdown link goes: the browser for web and mail, the reader for readable files next to
/// `file`, the folder for anything else. No link ever runs a file.
pub fn link_target(file: &Path, url: &str) -> LinkTarget {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return LinkTarget::Web(url.to_string());
    }
    if url.is_empty() || url.starts_with('#') || has_scheme(url) {
        return LinkTarget::Ignore;
    }
    let relative = percent_decode(url.split(['#', '?']).next().unwrap_or(""));
    let folder = file.parent().unwrap_or(Path::new(""));
    if let Some(dir) = relative.strip_suffix('/') {
        return LinkTarget::Reveal(folder.join(dir.replace('/', MAIN_SEPARATOR_STR)));
    }
    if relative.is_empty() {
        return LinkTarget::Ignore;
    }
    let target = folder.join(relative.replace('/', MAIN_SEPARATOR_STR));
    if Kind::from_path(&target, &[]).is_some() {
        LinkTarget::Read(target)
    } else {
        LinkTarget::Reveal(target)
    }
}

fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(at) if at > 1 => url[..at]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
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

/// Order matches the `kind` values in `ui/reader.slint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockKind {
    Heading,
    #[default]
    Paragraph,
    Item,
    Code,
    Quote,
    Rule,
    TableRow,
    Image,
}

impl BlockKind {
    pub fn index(self) -> i32 {
        self as i32
    }
}

/// One row of the Markdown view: `markup` goes to `StyledText`, `plain` is shown as is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockRow {
    pub kind: BlockKind,
    pub level: usize,
    pub marker: String,
    pub markup: String,
    pub plain: String,
    pub cells: Vec<String>,
    pub header: bool,
}

/// One row per block; a table becomes a header row and one row per line, all with the same columns.
pub fn block_rows(blocks: &[Block]) -> Vec<BlockRow> {
    let mut rows = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            Block::Heading { level, inline } => rows.push(BlockRow {
                kind: BlockKind::Heading,
                level: usize::from(*level),
                markup: inline.clone(),
                ..BlockRow::default()
            }),
            Block::Paragraph { inline } => rows.push(BlockRow {
                markup: inline.clone(),
                ..BlockRow::default()
            }),
            Block::Item {
                depth,
                marker,
                inline,
            } => rows.push(BlockRow {
                kind: BlockKind::Item,
                level: *depth,
                marker: marker.clone(),
                markup: inline.clone(),
                ..BlockRow::default()
            }),
            Block::Code { lang, text } => rows.push(BlockRow {
                kind: BlockKind::Code,
                marker: lang.clone(),
                plain: text.clone(),
                ..BlockRow::default()
            }),
            Block::Quote { depth, inline } => rows.push(BlockRow {
                kind: BlockKind::Quote,
                level: *depth,
                markup: inline.clone(),
                ..BlockRow::default()
            }),
            Block::Rule => rows.push(BlockRow {
                kind: BlockKind::Rule,
                ..BlockRow::default()
            }),
            Block::Table { header, rows: body } => {
                let columns = body
                    .iter()
                    .map(Vec::len)
                    .chain([header.len()])
                    .max()
                    .unwrap_or(0);
                let pad = |cells: &[String]| {
                    let mut cells = cells.to_vec();
                    cells.resize(columns, String::new());
                    cells
                };
                rows.push(BlockRow {
                    kind: BlockKind::TableRow,
                    cells: pad(header),
                    header: true,
                    ..BlockRow::default()
                });
                for row in body {
                    rows.push(BlockRow {
                        kind: BlockKind::TableRow,
                        cells: pad(row),
                        ..BlockRow::default()
                    });
                }
            }
            Block::Image { alt } => rows.push(BlockRow {
                kind: BlockKind::Image,
                plain: alt.clone(),
                ..BlockRow::default()
            }),
        }
    }
    rows
}

/// Order matches `encoding-name` in `ui/reader.slint`.
pub fn encoding_index(encoding: Encoding) -> i32 {
    match encoding {
        Encoding::Utf8 { bom: false } => 0,
        Encoding::Utf8 { bom: true } => 1,
        Encoding::Utf16Le => 2,
        Encoding::Utf16Be => 3,
        Encoding::Windows1252 => 4,
    }
}

pub fn wrap_by_default(path: &Path) -> bool {
    !matches!(
        Kind::from_path(path, &[]),
        Some(Kind::Plain { wrap: false })
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u32, height: u32, scale: f32) -> WorkArea {
        WorkArea {
            x: 0,
            y: 0,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn window_takes_most_of_the_work_area_up_to_a_limit() {
        assert_eq!(reader_size(&area(1366, 728, 1.0)), (956.0, 582.0));
        assert_eq!(reader_size(&area(1920, 1040, 1.0)), (1100.0, 832.0));
        assert_eq!(reader_size(&area(2560, 1400, 1.5)), (1100.0, 747.0));
    }

    #[test]
    fn display_lines_drop_the_final_break_and_cut_long_lines() {
        assert_eq!(display_lines("a\nb\n"), ["a", "b"]);
        assert_eq!(display_lines(""), [""]);
        let long = "é".repeat(MAX_LINE_BYTES);
        let shown = display_line(&long);
        assert!(shown.ends_with('…'));
        assert!(shown.len() <= MAX_LINE_BYTES + '…'.len_utf8());
        assert_eq!(longest(&["ab".into(), "éée".into()]), 3);
    }

    #[test]
    fn pages_are_requested_once_and_then_served() {
        let mut cache = PageCache::new(2);
        assert_eq!(cache.get(5), Lookup::Miss { request: Some(0) });
        assert_eq!(cache.get(6), Lookup::Miss { request: None });
        cache.insert(0, vec!["zero".into(), "one".into()]);
        assert_eq!(cache.get(1), Lookup::Hit("one"));
        assert_eq!(cache.get(5), Lookup::Hit(""));
        assert_eq!(cache.get(PAGE_LINES), Lookup::Miss { request: Some(1) });
    }

    #[test]
    fn the_oldest_page_is_evicted() {
        let mut cache = PageCache::new(2);
        cache.insert(0, vec!["a".into()]);
        cache.insert(1, vec!["b".into()]);
        cache.insert(2, vec!["c".into()]);
        assert_eq!(cache.get(0), Lookup::Miss { request: Some(0) });
        assert_eq!(cache.get(2 * PAGE_LINES), Lookup::Hit("c"));
    }

    #[test]
    fn web_links_open_in_the_browser() {
        let file = Path::new("notas").join("README.md");
        assert_eq!(
            link_target(&file, "https://x.y/a"),
            LinkTarget::Web("https://x.y/a".into())
        );
        assert_eq!(
            link_target(&file, "MAILTO:a@b.c"),
            LinkTarget::Web("MAILTO:a@b.c".into())
        );
    }

    #[test]
    fn anchors_and_other_schemes_are_ignored() {
        let file = Path::new("notas").join("README.md");
        for url in [
            "",
            "#uso",
            "javascript:alert(1)",
            "file:///c:/x.md",
            "vscode://x",
        ] {
            assert_eq!(link_target(&file, url), LinkTarget::Ignore, "{url}");
        }
    }

    #[test]
    fn relative_links_open_in_the_reader_or_in_the_folder() {
        let folder = Path::new("notas");
        let file = folder.join("README.md");
        assert_eq!(
            link_target(&file, "docs/guia%20rapido.md#passo-2"),
            LinkTarget::Read(folder.join("docs").join("guia rapido.md"))
        );
        assert_eq!(
            link_target(&file, "setup.exe"),
            LinkTarget::Reveal(folder.join("setup.exe"))
        );
        assert_eq!(
            link_target(&file, "docs/"),
            LinkTarget::Reveal(folder.join("docs"))
        );
    }

    #[test]
    fn blocks_become_rows_and_tables_expand() {
        let rows = block_rows(&[
            Block::Heading {
                level: 2,
                inline: "T".into(),
            },
            Block::Item {
                depth: 2,
                marker: "1.".into(),
                inline: "x".into(),
            },
            Block::Table {
                header: vec!["a".into(), "b".into()],
                rows: vec![vec!["1".into()]],
            },
            Block::Image { alt: "logo".into() },
        ]);
        assert_eq!(rows.len(), 5);
        assert_eq!(
            (rows[0].kind, rows[0].level, rows[0].markup.as_str()),
            (BlockKind::Heading, 2, "T")
        );
        assert_eq!(
            (rows[1].kind, rows[1].level, rows[1].marker.as_str()),
            (BlockKind::Item, 2, "1.")
        );
        assert!(rows[2].header);
        assert_eq!(rows[3].cells, ["1", ""]);
        assert_eq!(
            (rows[4].kind, rows[4].plain.as_str()),
            (BlockKind::Image, "logo")
        );
    }

    #[test]
    fn encodings_and_wrapping_map_to_the_window() {
        assert_eq!(encoding_index(Encoding::Utf8 { bom: false }), 0);
        assert_eq!(encoding_index(Encoding::Windows1252), 4);
        assert!(wrap_by_default(Path::new("a.md")));
        assert!(!wrap_by_default(Path::new("a.log")));
        assert!(wrap_by_default(Path::new("README")));
    }
}
