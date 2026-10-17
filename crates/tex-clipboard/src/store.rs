use std::fmt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::crypto::{Key, open, seal};
use crate::retention::{self, Retention, RowFacts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text = 0,
    Image = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryFilter {
    #[default]
    All,
    Favorites,
    Text,
    Images,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Text(String),
    Image(Vec<u8>),
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: i64,
    pub kind: Kind,
    pub copied_at: i64,
    pub size: u64,
    pub favorite: bool,
    pub name: Option<String>,
    pub preview: Preview,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Image {
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewEntry {
    Text(String),
    Image {
        png: Vec<u8>,
        width: u32,
        height: u32,
        thumbnail: Vec<u8>,
    },
}

#[derive(Debug)]
pub enum OpenError {
    WrongKey,
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for OpenError {
    fn from(error: rusqlite::Error) -> OpenError {
        OpenError::Sql(error)
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::WrongKey => f.write_str("the key does not open this history"),
            OpenError::Sql(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    TooLarge,
    NotText,
    Missing,
}

const SCHEMA_VERSION: i32 = 1;
const CHECK: &[u8] = b"tex-clipboard";
const SUMMARY_CHARS: usize = 200;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind INTEGER NOT NULL,
    copied_at INTEGER NOT NULL,
    size INTEGER NOT NULL,
    favorite INTEGER NOT NULL DEFAULT 0,
    name BLOB,
    summary BLOB NOT NULL,
    content BLOB NOT NULL,
    width INTEGER,
    height INTEGER
);
CREATE INDEX IF NOT EXISTS entries_copied_at ON entries (copied_at);
";

const COLUMNS: &str = "id, kind, copied_at, size, favorite, name, summary, width, height";

fn filter_sql(filter: HistoryFilter) -> &'static str {
    match filter {
        HistoryFilter::All => "1",
        HistoryFilter::Favorites => "favorite = 1",
        HistoryFilter::Text => "kind = 0",
        HistoryFilter::Images => "kind = 1",
    }
}

/// First characters of `text` on one line, whitespace runs collapsed into one space.
fn text_summary(text: &str) -> String {
    let collapsed: String = text
        .split_whitespace()
        .flat_map(|word| std::iter::once(' ').chain(word.chars()))
        .skip(1)
        .take(SUMMARY_CHARS)
        .collect();
    collapsed.trim_end().to_string()
}

struct Raw {
    id: i64,
    kind: Kind,
    copied_at: i64,
    size: u64,
    favorite: bool,
    name: Option<Vec<u8>>,
    summary: Vec<u8>,
    width: Option<u32>,
    height: Option<u32>,
}

impl Raw {
    fn into_summary(self, name: Option<String>, preview: Preview) -> Summary {
        Summary {
            id: self.id,
            kind: self.kind,
            copied_at: self.copied_at,
            size: self.size,
            favorite: self.favorite,
            name,
            preview,
            width: self.width,
            height: self.height,
        }
    }
}

fn kind_from(value: i64) -> Kind {
    if value == Kind::Image as i64 {
        Kind::Image
    } else {
        Kind::Text
    }
}

fn read_raw(row: &Row) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get(0)?,
        kind: kind_from(row.get(1)?),
        copied_at: row.get(2)?,
        size: row.get::<_, i64>(3)?.max(0) as u64,
        favorite: row.get(4)?,
        name: row.get(5)?,
        summary: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
    })
}

pub struct Store {
    conn: Connection,
    key: Key,
}

impl Store {
    pub fn open(path: &Path, key: Key) -> Result<Store, OpenError> {
        Store::init(Connection::open(path)?, key)
    }

    pub fn open_in_memory(key: Key) -> Result<Store, OpenError> {
        Store::init(Connection::open_in_memory()?, key)
    }

    fn init(conn: Connection, key: Key) -> Result<Store, OpenError> {
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        let check: Option<Vec<u8>> = conn
            .query_row("SELECT value FROM meta WHERE key = 'check'", [], |row| {
                row.get(0)
            })
            .optional()?;
        match check {
            Some(sealed) if open(&key, &sealed).as_deref() != Some(CHECK) => {
                return Err(OpenError::WrongKey);
            }
            Some(_) => {}
            None => {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('check', ?1)",
                    [seal(&key, CHECK)],
                )?;
            }
        }
        Ok(Store { conn, key })
    }

    fn open_text(&self, sealed: &[u8]) -> Option<String> {
        String::from_utf8(open(&self.key, sealed)?).ok()
    }

    fn readable(&self, raw: &Raw) -> Option<(Option<String>, Preview)> {
        let name = match &raw.name {
            Some(sealed) => Some(self.open_text(sealed)?),
            None => None,
        };
        let preview = match raw.kind {
            Kind::Text => Preview::Text(self.open_text(&raw.summary)?),
            Kind::Image => Preview::Image(open(&self.key, &raw.summary)?),
        };
        Some((name, preview))
    }

    fn to_summary(&self, raw: Raw) -> Summary {
        let (name, preview) = self.readable(&raw).unwrap_or((None, Preview::Unreadable));
        raw.into_summary(name, preview)
    }

    pub fn insert(&mut self, entry: &NewEntry, copied_at: i64) -> rusqlite::Result<Summary> {
        let (kind, summary, preview, content, size, width, height) = match entry {
            NewEntry::Text(text) => {
                let summary = text_summary(text);
                (
                    Kind::Text,
                    seal(&self.key, summary.as_bytes()),
                    Preview::Text(summary),
                    seal(&self.key, text.as_bytes()),
                    text.len(),
                    None,
                    None,
                )
            }
            NewEntry::Image {
                png,
                width,
                height,
                thumbnail,
            } => (
                Kind::Image,
                seal(&self.key, thumbnail),
                Preview::Image(thumbnail.clone()),
                seal(&self.key, png),
                png.len(),
                Some(*width),
                Some(*height),
            ),
        };
        self.conn.execute(
            "INSERT INTO entries (kind, copied_at, size, summary, content, width, height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                kind as i64,
                copied_at,
                size as i64,
                summary,
                content,
                width,
                height
            ],
        )?;
        Ok(Summary {
            id: self.conn.last_insert_rowid(),
            kind,
            copied_at,
            size: size as u64,
            favorite: false,
            name: None,
            preview,
            width,
            height,
        })
    }

    /// Newest first, older than `before` (an id) when given.
    pub fn page(
        &self,
        filter: HistoryFilter,
        before: Option<i64>,
        limit: usize,
    ) -> rusqlite::Result<Vec<Summary>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM entries WHERE {} AND id < ?1 ORDER BY id DESC LIMIT ?2",
            filter_sql(filter)
        );
        let mut statement = self.conn.prepare(&sql)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = statement.query_map(params![before.unwrap_or(i64::MAX), limit], read_raw)?;
        rows.map(|raw| raw.map(|raw| self.to_summary(raw)))
            .collect()
    }

    /// `None` when `cancelled()` turned true midway.
    pub fn search(
        &self,
        query: &str,
        filter: HistoryFilter,
        limit: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> rusqlite::Result<Option<Vec<Summary>>> {
        let query = tex_index::fold(query.trim());
        if query.is_empty() {
            return self.page(filter, None, limit).map(Some);
        }
        let sql = format!(
            "SELECT {COLUMNS}, CASE WHEN kind = 0 THEN content END FROM entries
             WHERE {} ORDER BY id DESC",
            filter_sql(filter)
        );
        let mut statement = self.conn.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let mut found = Vec::new();
        while found.len() < limit {
            if cancelled() {
                return Ok(None);
            }
            let Some(row) = rows.next()? else {
                break;
            };
            let raw = read_raw(row)?;
            let content: Option<Vec<u8>> = row.get(9)?;
            let Some((name, preview)) = self.readable(&raw) else {
                continue;
            };
            let matches = name
                .as_deref()
                .is_some_and(|name| tex_index::fold(name).contains(&query))
                || content
                    .and_then(|sealed| self.open_text(&sealed))
                    .is_some_and(|text| tex_index::fold(&text).contains(&query));
            if matches {
                found.push(raw.into_summary(name, preview));
            }
        }
        Ok(Some(found))
    }

    pub fn summary(&self, id: i64) -> rusqlite::Result<Option<Summary>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM entries WHERE id = ?1"),
                [id],
                read_raw,
            )
            .optional()?;
        Ok(raw.map(|raw| self.to_summary(raw)))
    }

    /// `Ok(None)` when the row is gone or does not decrypt.
    pub fn content(&self, id: i64) -> rusqlite::Result<Option<Content>> {
        let row = self
            .conn
            .query_row(
                "SELECT kind, content, width, height FROM entries WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        kind_from(row.get(0)?),
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Option<u32>>(2)?,
                        row.get::<_, Option<u32>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, sealed, width, height)) = row else {
            return Ok(None);
        };
        Ok(match kind {
            Kind::Text => self.open_text(&sealed).map(Content::Text),
            Kind::Image => open(&self.key, &sealed).map(|png| Content::Image {
                png,
                width: width.unwrap_or(0),
                height: height.unwrap_or(0),
            }),
        })
    }

    pub fn set_favorite(&mut self, id: i64, favorite: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE entries SET favorite = ?2 WHERE id = ?1",
            params![id, favorite],
        )?;
        Ok(())
    }

    pub fn rename(&mut self, id: i64, name: Option<&str>) -> rusqlite::Result<()> {
        let sealed = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| seal(&self.key, name.as_bytes()));
        self.conn.execute(
            "UPDATE entries SET name = ?2 WHERE id = ?1",
            params![id, sealed],
        )?;
        Ok(())
    }

    pub fn edit_text(
        &mut self,
        id: i64,
        text: &str,
        max_bytes: u64,
    ) -> rusqlite::Result<Result<Summary, EditError>> {
        let kind: Option<i64> = self
            .conn
            .query_row("SELECT kind FROM entries WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        match kind.map(kind_from) {
            None => return Ok(Err(EditError::Missing)),
            Some(Kind::Image) => return Ok(Err(EditError::NotText)),
            Some(Kind::Text) => {}
        }
        if text.len() as u64 > max_bytes {
            return Ok(Err(EditError::TooLarge));
        }
        self.conn.execute(
            "UPDATE entries SET content = ?2, summary = ?3, size = ?4 WHERE id = ?1",
            params![
                id,
                seal(&self.key, text.as_bytes()),
                seal(&self.key, text_summary(text).as_bytes()),
                text.len() as i64
            ],
        )?;
        Ok(self.summary(id)?.ok_or(EditError::Missing))
    }

    pub fn delete(&mut self, id: i64) -> rusqlite::Result<()> {
        self.delete_ids(&[id])
    }

    /// Ids removed.
    pub fn clear_non_favorites(&mut self) -> rusqlite::Result<Vec<i64>> {
        let ids = self
            .conn
            .prepare("DELETE FROM entries WHERE favorite = 0 RETURNING id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        self.vacuum()?;
        Ok(ids)
    }

    /// Bytes used by non-favorites.
    pub fn usage(&self) -> rusqlite::Result<u64> {
        let total: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM entries WHERE favorite = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(total.max(0) as u64)
    }

    /// Ids removed.
    pub fn apply_retention(&mut self, policy: Retention, now: i64) -> rusqlite::Result<Vec<i64>> {
        let facts = self.facts()?;
        let victims = match policy {
            Retention::Time { days } => retention::expired(&facts, days, now),
            Retention::Storage { limit_bytes } => retention::storage_victims(&facts, limit_bytes),
        };
        self.delete_ids(&victims)?;
        Ok(victims)
    }

    pub fn next_expiry(&self, policy: Retention) -> rusqlite::Result<Option<i64>> {
        match policy {
            Retention::Time { days } => Ok(retention::next_expiry(&self.facts()?, days)),
            Retention::Storage { .. } => Ok(None),
        }
    }

    fn facts(&self) -> rusqlite::Result<Vec<RowFacts>> {
        self.conn
            .prepare("SELECT id, copied_at, size, favorite FROM entries ORDER BY id")?
            .query_map([], |row| {
                Ok(RowFacts {
                    id: row.get(0)?,
                    copied_at: row.get(1)?,
                    size: row.get::<_, i64>(2)?.max(0) as u64,
                    favorite: row.get(3)?,
                })
            })?
            .collect()
    }

    fn delete_ids(&mut self, ids: &[i64]) -> rusqlite::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let transaction = self.conn.transaction()?;
        {
            let mut statement = transaction.prepare("DELETE FROM entries WHERE id = ?1")?;
            for id in ids {
                statement.execute([id])?;
            }
        }
        transaction.commit()?;
        self.vacuum()
    }

    /// Runs `incremental_vacuum` to completion: each step frees only some pages.
    fn vacuum(&self) -> rusqlite::Result<()> {
        let mut statement = self.conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use super::*;
    use crate::crypto::new_key;
    use crate::image::{Rgba, encode_png};
    use crate::retention::DAY_MS;

    const NOW: i64 = 1_800_000_000_000;

    fn store() -> Store {
        Store::open_in_memory(new_key()).unwrap()
    }

    fn text(value: &str) -> NewEntry {
        NewEntry::Text(value.to_string())
    }

    fn image() -> NewEntry {
        let png = encode_png(&Rgba {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 128],
        });
        NewEntry::Image {
            png,
            width: 2,
            height: 1,
            thumbnail: vec![1, 2, 3],
        }
    }

    fn ids(rows: &[Summary]) -> Vec<i64> {
        rows.iter().map(|row| row.id).collect()
    }

    fn page_ids(store: &Store, filter: HistoryFilter) -> Vec<i64> {
        ids(&store.page(filter, None, 100).unwrap())
    }

    fn search_ids(store: &Store, query: &str, filter: HistoryFilter) -> Vec<i64> {
        ids(&store
            .search(query, filter, 100, &|| false)
            .unwrap()
            .unwrap())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tex-clipboard-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn identical_copies_are_separate_rows() {
        let mut store = store();
        let first = store.insert(&text("same"), NOW).unwrap();
        let second = store.insert(&text("same"), NOW + 1).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(page_ids(&store, HistoryFilter::All), [second.id, first.id]);
        assert_eq!(second.kind, Kind::Text);
        assert_eq!(second.copied_at, NOW + 1);
        assert_eq!(second.size, 4);
        assert!(!second.favorite);
        assert_eq!(second.name, None);
        assert_eq!(second.preview, Preview::Text("same".into()));
        assert_eq!(store.summary(second.id).unwrap(), Some(second));
    }

    #[test]
    fn actions_touch_only_one_row() {
        let mut store = store();
        let first = store.insert(&text("same"), NOW).unwrap();
        let second = store.insert(&text("same"), NOW).unwrap();
        store.set_favorite(first.id, true).unwrap();
        store.rename(first.id, Some("  Nome  ")).unwrap();
        store.edit_text(first.id, "changed", 100).unwrap().unwrap();
        assert_eq!(store.summary(second.id).unwrap(), Some(second.clone()));
        assert_eq!(
            store.content(second.id).unwrap(),
            Some(Content::Text("same".into()))
        );
        let edited = store.summary(first.id).unwrap().unwrap();
        assert!(edited.favorite);
        assert_eq!(edited.name.as_deref(), Some("Nome"));
        assert_eq!(edited.preview, Preview::Text("changed".into()));
        store.delete(first.id).unwrap();
        assert_eq!(store.summary(first.id).unwrap(), None);
        assert_eq!(store.summary(second.id).unwrap(), Some(second.clone()));
        assert_eq!(page_ids(&store, HistoryFilter::All), [second.id]);
    }

    #[test]
    fn content_comes_back_as_inserted() {
        let mut store = store();
        let text_row = store.insert(&text("olá\nmundo"), NOW).unwrap();
        let image_entry = image();
        let image_row = store.insert(&image_entry, NOW).unwrap();
        assert_eq!(
            store.content(text_row.id).unwrap(),
            Some(Content::Text("olá\nmundo".into()))
        );
        let NewEntry::Image { png, .. } = image_entry else {
            unreachable!()
        };
        assert_eq!(image_row.kind, Kind::Image);
        assert_eq!(image_row.size, png.len() as u64);
        assert_eq!(image_row.preview, Preview::Image(vec![1, 2, 3]));
        assert_eq!((image_row.width, image_row.height), (Some(2), Some(1)));
        assert_eq!(
            store.content(image_row.id).unwrap(),
            Some(Content::Image {
                png,
                width: 2,
                height: 1
            })
        );
        assert_eq!(store.content(9_999).unwrap(), None);
    }

    #[test]
    fn text_summary_collapses_whitespace_and_keeps_200_chars() {
        let mut store = store();
        let row = store.insert(&text("  um\n\n dois\t três  "), NOW).unwrap();
        assert_eq!(row.preview, Preview::Text("um dois três".into()));
        assert_eq!(row.size, "  um\n\n dois\t três  ".len() as u64);
        let long = format!("{} fim", "á".repeat(300));
        let row = store.insert(&text(&long), NOW).unwrap();
        assert_eq!(row.preview, Preview::Text("á".repeat(200)));
        let row = store
            .insert(&text(&format!("{} b", "a".repeat(199))), NOW)
            .unwrap();
        assert_eq!(row.preview, Preview::Text("a".repeat(199)));
    }

    #[test]
    fn blank_name_is_stored_as_none() {
        let mut store = store();
        let row = store.insert(&text("x"), NOW).unwrap();
        store.rename(row.id, Some("nome")).unwrap();
        store.rename(row.id, Some("  ")).unwrap();
        assert_eq!(store.summary(row.id).unwrap().unwrap().name, None);
        store.rename(row.id, Some("nome")).unwrap();
        store.rename(row.id, None).unwrap();
        assert_eq!(store.summary(row.id).unwrap().unwrap().name, None);
    }

    #[test]
    fn pages_and_filters() {
        let mut store = store();
        let a = store.insert(&text("a"), NOW).unwrap().id;
        let b = store.insert(&image(), NOW).unwrap().id;
        let c = store.insert(&text("c"), NOW).unwrap().id;
        let d = store.insert(&image(), NOW).unwrap().id;
        store.set_favorite(a, true).unwrap();
        store.set_favorite(d, true).unwrap();
        assert_eq!(
            ids(&store.page(HistoryFilter::All, None, 2).unwrap()),
            [d, c]
        );
        assert_eq!(
            ids(&store.page(HistoryFilter::All, Some(c), 2).unwrap()),
            [b, a]
        );
        assert!(
            store
                .page(HistoryFilter::All, Some(a), 2)
                .unwrap()
                .is_empty()
        );
        assert_eq!(page_ids(&store, HistoryFilter::Favorites), [d, a]);
        assert_eq!(page_ids(&store, HistoryFilter::Text), [c, a]);
        assert_eq!(page_ids(&store, HistoryFilter::Images), [d, b]);
    }

    #[test]
    fn search_folds_case_and_accents() {
        let mut store = store();
        let found = store.insert(&text("Uma ação rápida"), NOW).unwrap().id;
        store.insert(&text("outra coisa"), NOW).unwrap();
        assert_eq!(search_ids(&store, "ACAO", HistoryFilter::All), [found]);
        assert_eq!(search_ids(&store, "  rápida ", HistoryFilter::All), [found]);
        assert!(search_ids(&store, "ação", HistoryFilter::Images).is_empty());
    }

    #[test]
    fn search_by_name_and_images_only_by_name() {
        let mut store = store();
        let named_text = store.insert(&text("conteudo"), NOW).unwrap().id;
        let named_image = store.insert(&image(), NOW).unwrap().id;
        let other_image = store.insert(&image(), NOW).unwrap().id;
        store.rename(named_text, Some("Relatório")).unwrap();
        store.rename(named_image, Some("Logo Relatorio")).unwrap();
        assert_eq!(
            search_ids(&store, "relatorio", HistoryFilter::All),
            [named_image, named_text]
        );
        assert_eq!(
            search_ids(&store, "relatorio", HistoryFilter::Images),
            [named_image]
        );
        assert!(search_ids(&store, "png", HistoryFilter::All).is_empty());
        assert_eq!(
            search_ids(&store, "", HistoryFilter::Images),
            [other_image, named_image]
        );
    }

    #[test]
    fn search_respects_limit_and_cancellation() {
        let mut store = store();
        let mut all = Vec::new();
        for _ in 0..10 {
            all.push(store.insert(&text("match"), NOW).unwrap().id);
        }
        all.reverse();
        let limited = store
            .search("match", HistoryFilter::All, 3, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(ids(&limited), all[..3]);
        let calls = Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(
            store
                .search("match", HistoryFilter::All, 100, &cancelled)
                .unwrap(),
            None
        );
    }

    #[test]
    fn edit_text_refusals_and_kept_timestamp() {
        let mut store = store();
        let row = store.insert(&text("abc"), NOW).unwrap();
        let picture = store.insert(&image(), NOW).unwrap();
        assert_eq!(
            store.edit_text(row.id, "too long", 7).unwrap(),
            Err(EditError::TooLarge)
        );
        assert_eq!(
            store.edit_text(picture.id, "x", 100).unwrap(),
            Err(EditError::NotText)
        );
        assert_eq!(
            store.edit_text(9_999, "x", 100).unwrap(),
            Err(EditError::Missing)
        );
        let edited = store.edit_text(row.id, "exactly8", 8).unwrap().unwrap();
        assert_eq!(edited.copied_at, NOW);
        assert_eq!(edited.size, 8);
        assert_eq!(edited.preview, Preview::Text("exactly8".into()));
        assert_eq!(
            store.content(row.id).unwrap(),
            Some(Content::Text("exactly8".into()))
        );
    }

    #[test]
    fn usage_ignores_favorites() {
        let mut store = store();
        assert_eq!(store.usage().unwrap(), 0);
        store.insert(&text("12345"), NOW).unwrap();
        let favorite = store.insert(&text("1234567890"), NOW).unwrap();
        store.set_favorite(favorite.id, true).unwrap();
        assert_eq!(store.usage().unwrap(), 5);
    }

    #[test]
    fn time_retention_removes_only_expired_non_favorites() {
        let mut store = store();
        let old = store.insert(&text("old"), NOW - 8 * DAY_MS).unwrap().id;
        let old_favorite = store.insert(&text("fav"), NOW - 8 * DAY_MS).unwrap().id;
        let recent = store.insert(&text("new"), NOW - DAY_MS).unwrap().id;
        store.set_favorite(old_favorite, true).unwrap();
        let policy = Retention::Time { days: 7 };
        assert_eq!(
            store.next_expiry(policy).unwrap(),
            Some(NOW - 8 * DAY_MS + 7 * DAY_MS)
        );
        assert_eq!(store.apply_retention(policy, NOW).unwrap(), [old]);
        assert_eq!(page_ids(&store, HistoryFilter::All), [recent, old_favorite]);
        assert_eq!(store.next_expiry(policy).unwrap(), Some(NOW + 6 * DAY_MS));
        assert_eq!(
            store
                .next_expiry(Retention::Storage { limit_bytes: 10 })
                .unwrap(),
            None
        );
        store.delete(recent).unwrap();
        assert_eq!(store.next_expiry(policy).unwrap(), None);
    }

    #[test]
    fn storage_retention_removes_oldest_until_80_percent() {
        let mut store = store();
        let first = store.insert(&text(&"a".repeat(30)), NOW).unwrap().id;
        let favorite = store.insert(&text(&"f".repeat(500)), NOW).unwrap().id;
        let second = store.insert(&text(&"b".repeat(30)), NOW + 1).unwrap().id;
        store.set_favorite(favorite, true).unwrap();
        let policy = Retention::Storage { limit_bytes: 100 };
        assert!(store.apply_retention(policy, NOW).unwrap().is_empty());
        let third = store.insert(&text(&"c".repeat(30)), NOW + 2).unwrap().id;
        assert_eq!(store.apply_retention(policy, NOW).unwrap(), [first]);
        assert_eq!(
            page_ids(&store, HistoryFilter::All),
            [third, second, favorite]
        );
        assert_eq!(store.usage().unwrap(), 60);
    }

    #[test]
    fn clear_keeps_favorites() {
        let mut store = store();
        let a = store.insert(&text("a"), NOW).unwrap().id;
        let b = store.insert(&text("b"), NOW).unwrap().id;
        let c = store.insert(&image(), NOW).unwrap().id;
        store.set_favorite(b, true).unwrap();
        let mut removed = store.clear_non_favorites().unwrap();
        removed.sort();
        assert_eq!(removed, [a, c]);
        assert_eq!(page_ids(&store, HistoryFilter::All), [b]);
    }

    #[test]
    fn reopening_needs_the_same_key() {
        let dir = temp_dir("reopen");
        let path = dir.join("clipboard.db");
        let key = new_key();
        let id = {
            let mut store = Store::open(&path, key.clone()).unwrap();
            let id = store.insert(&text("guardado"), NOW).unwrap().id;
            store.rename(id, Some("nome")).unwrap();
            id
        };
        assert!(matches!(
            Store::open(&path, new_key()),
            Err(OpenError::WrongKey)
        ));
        let store = Store::open(&path, key).unwrap();
        let auto_vacuum: i64 = store
            .conn
            .pragma_query_value(None, "auto_vacuum", |row| row.get(0))
            .unwrap();
        assert_eq!(auto_vacuum, 2);
        let row = store.summary(id).unwrap().unwrap();
        assert_eq!(row.name.as_deref(), Some("nome"));
        assert_eq!(
            store.content(id).unwrap(),
            Some(Content::Text("guardado".into()))
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_fields_become_unreadable() {
        let mut store = store();
        let good = store.insert(&text("bom"), NOW).unwrap();
        let bad = store.insert(&text("ruim"), NOW).unwrap().id;
        store.rename(bad, Some("ruim")).unwrap();
        store
            .conn
            .execute(
                "UPDATE entries SET summary = x'00', content = x'00' WHERE id = ?1",
                [bad],
            )
            .unwrap();
        let rows = store.page(HistoryFilter::All, None, 10).unwrap();
        assert_eq!(rows[0].id, bad);
        assert_eq!(rows[0].preview, Preview::Unreadable);
        assert_eq!(rows[0].name, None);
        assert_eq!(rows[1], good);
        assert_eq!(store.content(bad).unwrap(), None);
        assert_eq!(
            store.content(good.id).unwrap(),
            Some(Content::Text("bom".into()))
        );
        assert_eq!(
            search_ids(&store, "ruim", HistoryFilter::All),
            Vec::<i64>::new()
        );
        assert_eq!(search_ids(&store, "bom", HistoryFilter::All), [good.id]);
        store.delete(bad).unwrap();
        assert_eq!(page_ids(&store, HistoryFilter::All), [good.id]);
    }
}
