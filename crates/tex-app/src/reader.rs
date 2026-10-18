use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tex_platform::WorkArea;
use tex_reader::{Block, Encoding, Kind, MAX_LINE_BYTES, Stamp};

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

/// How many lines `display_lines` gives, without building them.
pub fn line_count(text: &str) -> usize {
    text.strip_suffix('\n')
        .unwrap_or(text)
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        + 1
}

/// The widest line in columns, a tab counting as four.
pub fn longest(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum())
        .max()
        .unwrap_or(0)
}

/// Whether two paths name the same file once `.` and `..` are resolved, ignoring case on Windows.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        normalized(a).to_string_lossy().to_lowercase()
            == normalized(b).to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized(a) == normalized(b)
    }
}

fn normalized(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                Some(Component::RootDir) => {}
                _ => parts.push(part),
            },
            _ => parts.push(part),
        }
    }
    parts.iter().collect()
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

pub const MAX_HITS: usize = 10_000;
const CANCEL_EVERY: usize = 4096;

/// The text a reader sees in a block's markup: escapes resolved, emphasis, code and link syntax dropped.
pub fn plain_text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut chars = markup.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\n') => out.push(' '),
                Some(next) => out.push(next),
                None => {}
            },
            '*' | '~' | '`' | '[' => {}
            ']' if chars.peek() == Some(&'(') => {
                chars.next();
                if chars.peek() == Some(&'<') {
                    for c in chars.by_ref() {
                        if c == '>' {
                            break;
                        }
                    }
                }
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
            }
            ']' => {}
            _ => out.push(c),
        }
    }
    out
}

/// What the search compares for one Markdown row.
pub fn search_text(row: &BlockRow) -> String {
    match row.kind {
        BlockKind::Code | BlockKind::Image => row.plain.clone(),
        BlockKind::TableRow => row
            .cells
            .iter()
            .map(|cell| plain_text(cell))
            .collect::<Vec<_>>()
            .join(" "),
        _ => plain_text(&row.markup),
    }
}

/// Indexes (plus `offset`) of the rows containing `query`, ignoring case and accents, at most
/// `MAX_HITS`; `None` when `cancel` was set.
pub fn find_rows(
    rows: &[String],
    query: &str,
    offset: usize,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    let query = tex_index::fold(query.trim());
    let mut hits = Vec::new();
    if query.is_empty() {
        return Some(hits);
    }
    for (at, row) in rows.iter().enumerate() {
        if at % CANCEL_EVERY == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        if tex_index::fold(row).contains(&query) {
            hits.push(offset + at);
            if hits.len() == MAX_HITS {
                break;
            }
        }
    }
    Some(hits)
}

/// The next (or previous) hit, wrapping around; the first (or last) when none is current.
pub fn step_hit(count: usize, current: Option<usize>, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(at), true) => (at + 1) % count,
        (Some(at), false) => (at + count - 1) % count,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveCheck {
    Write,
    AskOutside,
    AskLossy,
}

/// Whether a save can go ahead, or must first ask about an outside change or lost characters.
pub fn save_check(changed_outside: bool, fits: bool, overwrite: bool, lossy: bool) -> SaveCheck {
    if changed_outside && !overwrite {
        SaveCheck::AskOutside
    } else if !fits && !lossy {
        SaveCheck::AskLossy
    } else {
        SaveCheck::Write
    }
}

/// Whether the file's stamp is now `current` after being `known`, unless the user already accepted
/// overwriting that very version (`acknowledged`).
pub fn changed_outside(known: Stamp, current: Option<Stamp>, acknowledged: Option<Stamp>) -> bool {
    current.is_some_and(|current| current != known && acknowledged != Some(current))
}

/// Whether saving must write: the document has changes, or the file is gone or not the one loaded.
pub fn needs_write(dirty: bool, known: Stamp, current: Option<Stamp>) -> bool {
    dirty || current != Some(known)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outside {
    Nothing,
    Reload,
    Conflict,
    Deleted,
}

/// What to do when the window comes back and the file's stamp is `current` (`None`: it is gone).
pub fn outside_change(known: Stamp, current: Option<Stamp>, dirty: bool) -> Outside {
    match current {
        None => Outside::Deleted,
        Some(current) if current == known => Outside::Nothing,
        Some(_) if dirty => Outside::Conflict,
        Some(_) => Outside::Reload,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use tex_reader::Stamp;

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
    fn tabs_count_as_four_columns() {
        assert_eq!(longest(&["\tab".into(), "abcde".into()]), 6);
    }

    #[test]
    fn line_count_matches_the_displayed_lines() {
        for text in ["", "a", "a\nb\n", "a\n\n", "\n", "a\nb"] {
            assert_eq!(line_count(text), display_lines(text).len(), "{text:?}");
        }
        assert_eq!(line_count(""), 1);
        assert_eq!(line_count("a\nb\n"), 2);
    }

    #[test]
    fn same_file_ignores_dots_and_parent_steps() {
        let readme = Path::new("notas").join("README.md");
        assert!(same_file(
            &Path::new("notas").join("docs").join("..").join("README.md"),
            &readme
        ));
        assert!(same_file(
            &Path::new(".").join("notas").join("README.md"),
            &readme
        ));
        assert!(!same_file(&Path::new("notas").join("LEIAME.md"), &readme));
        assert!(!same_file(&Path::new("docs").join("README.md"), &readme));
    }

    #[test]
    fn same_file_follows_the_case_rule_of_the_system() {
        let readme = Path::new("notas").join("README.md");
        let lower = Path::new("Notas").join("readme.MD");
        assert_eq!(same_file(&lower, &readme), cfg!(windows));
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

    #[test]
    fn plain_text_drops_markup_and_escapes() {
        assert_eq!(
            plain_text(r"a **b** \*c\* [d](<http://x.y/>) `e` ~~f~~"),
            "a b *c* d e f"
        );
        assert_eq!(plain_text(r"1 \= 2"), "1 = 2");
    }

    #[test]
    fn search_text_reads_every_kind_of_row() {
        let rows = block_rows(&[
            Block::Paragraph {
                inline: r"**Olá**\!".into(),
            },
            Block::Code {
                lang: "rs".into(),
                text: "fn main()".into(),
            },
            Block::Table {
                header: vec!["a".into(), "b".into()],
                rows: vec![],
            },
        ]);
        assert_eq!(search_text(&rows[0]), "Olá!");
        assert_eq!(search_text(&rows[1]), "fn main()");
        assert_eq!(search_text(&rows[2]), "a b");
    }

    #[test]
    fn find_rows_ignores_case_and_accents() {
        let rows: Vec<String> = ["Ação rápida", "nada", "ACAO lenta"]
            .map(String::from)
            .to_vec();
        let cancel = AtomicBool::new(false);
        assert_eq!(find_rows(&rows, "acao", 0, &cancel), Some(vec![0, 2]));
        assert_eq!(find_rows(&rows, "acao", 10, &cancel), Some(vec![10, 12]));
        assert_eq!(find_rows(&rows, "  ", 0, &cancel), Some(vec![]));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(find_rows(&rows, "acao", 0, &cancel), None);
    }

    #[test]
    fn steps_wrap_around_the_hits() {
        assert_eq!(step_hit(0, None, true), None);
        assert_eq!(step_hit(3, None, true), Some(0));
        assert_eq!(step_hit(3, None, false), Some(2));
        assert_eq!(step_hit(3, Some(2), true), Some(0));
        assert_eq!(step_hit(3, Some(0), false), Some(2));
    }

    #[test]
    fn saving_asks_before_overwriting_or_losing_characters() {
        assert_eq!(save_check(false, true, false, false), SaveCheck::Write);
        assert_eq!(save_check(true, true, false, false), SaveCheck::AskOutside);
        assert_eq!(save_check(true, false, true, false), SaveCheck::AskLossy);
        assert_eq!(save_check(true, false, true, true), SaveCheck::Write);
        assert_eq!(save_check(false, false, false, false), SaveCheck::AskLossy);
    }

    #[test]
    fn outside_changes_reload_or_conflict() {
        let known = Stamp {
            modified: None,
            len: 3,
        };
        let other = Stamp {
            modified: None,
            len: 4,
        };
        assert_eq!(outside_change(known, Some(known), false), Outside::Nothing);
        assert_eq!(outside_change(known, Some(other), false), Outside::Reload);
        assert_eq!(outside_change(known, Some(other), true), Outside::Conflict);
        assert_eq!(outside_change(known, None, false), Outside::Deleted);
    }

    fn stamp(len: u64) -> Stamp {
        Stamp {
            modified: None,
            len,
        }
    }

    #[test]
    fn an_acknowledged_outside_change_is_not_asked_again() {
        let (known, other, third) = (stamp(3), stamp(4), stamp(5));
        assert!(!changed_outside(known, Some(known), None));
        assert!(changed_outside(known, Some(other), None));
        assert!(!changed_outside(known, Some(other), Some(other)));
        assert!(changed_outside(known, Some(third), Some(other)));
        assert!(!changed_outside(known, None, None));
    }

    #[test]
    fn a_clean_document_of_an_unchanged_file_is_not_written() {
        let (known, other) = (stamp(3), stamp(4));
        assert!(!needs_write(false, known, Some(known)));
        assert!(needs_write(true, known, Some(known)));
        assert!(needs_write(false, known, Some(other)));
        assert!(needs_write(false, known, None));
    }
}
