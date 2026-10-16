use std::sync::Mutex;

use crate::entry::{Attributes, Entry, ROOT, Record};

const MAX_DEPTH: u16 = 512;
const MIN_COMPACT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lineage {
    pub visible: bool,
    pub in_home: bool,
    pub depth: u16,
}

pub struct VolumeIndex {
    letter: char,
    entries: Vec<Entry>,
    names: Vec<u8>,
    garbage: usize,
    live: usize,
    windows_dir: Option<Record>,
    lookup_cache: Mutex<Option<(String, Record)>>,
}

impl VolumeIndex {
    pub fn new(letter: char) -> Self {
        Self {
            letter: letter.to_ascii_uppercase(),
            entries: Vec::new(),
            names: Vec::new(),
            garbage: 0,
            live: 0,
            windows_dir: None,
            lookup_cache: Mutex::new(None),
        }
    }

    pub fn letter(&self) -> char {
        self.letter
    }

    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    pub fn garbage(&self) -> usize {
        self.garbage
    }

    pub fn name_bytes(&self) -> usize {
        self.names.len()
    }

    pub fn windows_dir(&self) -> Option<Record> {
        self.windows_dir
    }

    pub fn set_windows_dir(&mut self, record: Option<Record>) {
        self.windows_dir = record;
    }

    /// Creates the entry, or updates it in place on rename, move or attribute change.
    pub fn upsert(&mut self, record: Record, parent: Record, name: &str, attributes: Attributes) {
        assert!(
            name.len() <= u16::MAX as usize,
            "file name longer than 64 KiB"
        );
        let index = record as usize;
        if index >= self.entries.len() {
            self.entries.resize(index + 1, Entry::default());
        }
        let current = self.entries[index];
        let same_name = current.in_use() && self.entry_name(&current) == name.as_bytes();
        let (name_offset, name_len) = if same_name {
            (current.name_offset, current.name_len)
        } else {
            if current.in_use() {
                self.garbage += current.name_len as usize;
            } else {
                self.live += 1;
            }
            let offset = u32::try_from(self.names.len()).expect("name buffer over 4 GiB");
            self.names.extend_from_slice(name.as_bytes());
            (offset, name.len() as u16)
        };
        self.entries[index] = Entry {
            parent,
            name_offset,
            name_len,
            flags: attributes.to_flags(),
        };
        self.maybe_compact();
    }

    pub fn remove(&mut self, record: Record) {
        let Some(entry) = self.entries.get_mut(record as usize) else {
            return;
        };
        if !entry.in_use() {
            return;
        }
        entry.flags = 0;
        self.garbage += entry.name_len as usize;
        self.live -= 1;
        self.maybe_compact();
    }

    /// Rewrites the name buffer with only the names still in use.
    pub fn compact(&mut self) {
        let mut names = Vec::with_capacity(self.names.len() - self.garbage);
        for entry in self.entries.iter_mut().filter(|entry| entry.in_use()) {
            let start = entry.name_offset as usize;
            let offset = names.len() as u32;
            names.extend_from_slice(&self.names[start..start + entry.name_len as usize]);
            entry.name_offset = offset;
        }
        self.names = names;
        self.garbage = 0;
    }

    fn maybe_compact(&mut self) {
        if self.names.len() >= MIN_COMPACT_BYTES && self.garbage * 4 > self.names.len() {
            self.compact();
        }
    }

    pub fn name(&self, record: Record) -> Option<&str> {
        self.entry(record).map(|entry| self.entry_str(entry))
    }

    /// Full path starting at the drive letter. `None` if the record or an ancestor is not indexed,
    /// or the parent chain is deeper than 512 levels (a cycle).
    pub fn path(&self, record: Record) -> Option<String> {
        let mut parts = Vec::new();
        let mut current = record;
        while current != ROOT {
            if parts.len() >= MAX_DEPTH as usize {
                return None;
            }
            let entry = self.entry(current)?;
            parts.push(self.entry_str(entry));
            current = entry.parent;
        }
        let mut path = format!("{}:\\", self.letter);
        for (i, part) in parts.iter().rev().enumerate() {
            if i > 0 {
                path.push('\\');
            }
            path.push_str(part);
        }
        Some(path)
    }

    /// Record of a path on this volume, ignoring case. Scans every entry, so callers should cache it.
    pub fn lookup(&self, path: &str) -> Option<Record> {
        let normalized = path.replace('/', "\\");
        let normalized = normalized.trim_end_matches('\\');
        let mut chars = normalized.chars();
        let letter = chars.next()?;
        if !letter.eq_ignore_ascii_case(&self.letter) || chars.next() != Some(':') {
            return None;
        }
        let Some(last) = normalized[2..].split('\\').rfind(|part| !part.is_empty()) else {
            return Some(ROOT);
        };
        (0..self.entries.len() as Record).find(|&record| {
            self.name(record).is_some_and(|name| same_name(name, last))
                && self
                    .path(record)
                    .is_some_and(|full| same_name(&full, normalized))
        })
    }

    /// `lookup` with a one-entry cache, for the home folder that every query sends.
    pub(crate) fn cached_lookup(&self, path: &str) -> Option<Record> {
        let mut cache = self.lookup_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached, record)) = cache.as_ref()
            && cached == path
            && self.entry(*record).is_some()
        {
            return Some(*record);
        }
        let record = self.lookup(path)?;
        *cache = Some((path.to_owned(), record));
        Some(record)
    }

    /// Walks from `record` up to the root. `visible` is false if the record or an ancestor is hidden,
    /// system, the Windows folder, or an NTFS metadata entry at the root. `None` for orphans and cycles.
    pub(crate) fn lineage(&self, record: Record, home: Option<Record>) -> Option<Lineage> {
        let mut lineage = Lineage {
            visible: true,
            in_home: false,
            depth: 0,
        };
        let mut current = record;
        while current != ROOT {
            if lineage.depth >= MAX_DEPTH {
                return None;
            }
            let entry = self.entry(current)?;
            if Some(current) == home {
                lineage.in_home = true;
            }
            if entry.is_hidden_or_system()
                || Some(current) == self.windows_dir
                || (entry.parent == ROOT && is_root_metadata(self.entry_name(entry)))
            {
                lineage.visible = false;
            }
            lineage.depth += 1;
            current = entry.parent;
        }
        Some(lineage)
    }

    pub(crate) fn entry(&self, record: Record) -> Option<&Entry> {
        self.entries
            .get(record as usize)
            .filter(|entry| entry.in_use())
    }

    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(crate) fn entry_name(&self, entry: &Entry) -> &[u8] {
        let start = entry.name_offset as usize;
        &self.names[start..start + entry.name_len as usize]
    }

    fn entry_str(&self, entry: &Entry) -> &str {
        std::str::from_utf8(self.entry_name(entry)).expect("index names are valid UTF-8")
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
        || ((!a.is_ascii() || !b.is_ascii()) && a.to_lowercase() == b.to_lowercase())
}

fn is_root_metadata(name: &[u8]) -> bool {
    name.starts_with(b"$") || name.eq_ignore_ascii_case(b"System Volume Information")
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const FILE: Attributes = Attributes {
        dir: false,
        hidden: false,
        system: false,
    };
    pub(crate) const DIR: Attributes = Attributes {
        dir: true,
        hidden: false,
        system: false,
    };

    #[test]
    fn upsert_stores_new_entries() {
        let mut volume = VolumeIndex::new('c');
        volume.upsert(20, ROOT, "Users", DIR);
        volume.upsert(21, 20, "notes.txt", FILE);
        assert_eq!(volume.letter(), 'C');
        assert_eq!(volume.len(), 2);
        assert_eq!(volume.name(20), Some("Users"));
        assert_eq!(volume.name(21), Some("notes.txt"));
        assert_eq!(volume.name(22), None);
    }

    #[test]
    fn upsert_renames_and_moves_in_place() {
        let mut volume = VolumeIndex::new('C');
        volume.upsert(20, ROOT, "a", DIR);
        volume.upsert(21, ROOT, "b", DIR);
        volume.upsert(22, 20, "old.txt", FILE);
        volume.upsert(22, 21, "new.txt", FILE);
        assert_eq!(volume.len(), 3);
        assert_eq!(volume.name(22), Some("new.txt"));
        assert_eq!(volume.entry(22).map(|e| e.parent), Some(21));
        assert_eq!(volume.garbage(), "old.txt".len());
    }

    #[test]
    fn upsert_with_same_name_reuses_the_stored_name() {
        let mut volume = VolumeIndex::new('C');
        volume.upsert(20, ROOT, "doc.txt", FILE);
        volume.upsert(
            20,
            ROOT,
            "doc.txt",
            Attributes {
                hidden: true,
                ..FILE
            },
        );
        assert_eq!(volume.garbage(), 0);
        assert!(volume.entry(20).unwrap().is_hidden_or_system());
    }

    #[test]
    fn remove_frees_the_entry() {
        let mut volume = VolumeIndex::new('C');
        volume.upsert(20, ROOT, "gone.txt", FILE);
        volume.remove(20);
        volume.remove(20);
        volume.remove(999);
        assert_eq!(volume.len(), 0);
        assert_eq!(volume.name(20), None);
        assert_eq!(volume.garbage(), "gone.txt".len());
    }

    #[test]
    fn windows_dir_is_stored() {
        let mut volume = VolumeIndex::new('C');
        volume.set_windows_dir(Some(30));
        assert_eq!(volume.windows_dir(), Some(30));
    }

    pub(crate) fn sample() -> VolumeIndex {
        let mut volume = VolumeIndex::new('C');
        volume.upsert(20, ROOT, "Users", DIR);
        volume.upsert(21, 20, "Luís", DIR);
        volume.upsert(22, 21, "notes.txt", FILE);
        volume
    }

    #[test]
    fn path_walks_up_to_the_root() {
        let volume = sample();
        assert_eq!(volume.path(22).as_deref(), Some(r"C:\Users\Luís\notes.txt"));
        assert_eq!(volume.path(ROOT).as_deref(), Some(r"C:\"));
    }

    #[test]
    fn path_is_none_for_orphans_and_cycles() {
        let mut volume = sample();
        volume.upsert(30, 99, "orphan.txt", FILE);
        volume.upsert(31, 32, "a", DIR);
        volume.upsert(32, 31, "b", DIR);
        assert_eq!(volume.path(30), None);
        assert_eq!(volume.path(31), None);
    }

    #[test]
    fn lookup_ignores_case_and_trailing_separators() {
        let volume = sample();
        assert_eq!(volume.lookup(r"c:\users\LUÍS\"), Some(21));
        assert_eq!(volume.lookup("C:/Users/Luís/notes.txt"), Some(22));
        assert_eq!(volume.lookup(r"C:\"), Some(ROOT));
        assert_eq!(volume.lookup(r"D:\Users"), None);
        assert_eq!(volume.lookup(r"C:\Users\missing"), None);
    }

    #[test]
    fn compact_drops_unused_name_bytes() {
        let mut volume = sample();
        volume.upsert(22, 21, "renamed.txt", FILE);
        volume.compact();
        assert_eq!(volume.garbage(), 0);
        assert_eq!(
            volume.name_bytes(),
            "Users".len() + "Luís".len() + "renamed.txt".len()
        );
        assert_eq!(
            volume.path(22).as_deref(),
            Some(r"C:\Users\Luís\renamed.txt")
        );
    }

    #[test]
    fn heavy_renaming_triggers_compaction() {
        let mut volume = VolumeIndex::new('C');
        let long = "x".repeat(200);
        for i in 0..2_000 {
            volume.upsert(20, ROOT, &format!("{long}{i}"), FILE);
        }
        assert!(volume.name_bytes() < MIN_COMPACT_BYTES + 1024);
        assert_eq!(volume.name(20), Some(format!("{long}1999").as_str()));
    }

    #[test]
    fn lineage_reports_depth_and_home() {
        let volume = sample();
        let lineage = volume.lineage(22, Some(20)).unwrap();
        assert_eq!(lineage.depth, 3);
        assert!(lineage.in_home);
        assert!(lineage.visible);
        assert!(!volume.lineage(22, Some(99)).unwrap().in_home);
    }

    #[test]
    fn hidden_or_system_ancestor_hides_descendants() {
        let mut volume = sample();
        volume.upsert(
            23,
            21,
            "AppData",
            Attributes {
                hidden: true,
                ..DIR
            },
        );
        volume.upsert(24, 23, "config.json", FILE);
        assert!(!volume.lineage(24, None).unwrap().visible);
        assert!(volume.lineage(22, None).unwrap().visible);
    }

    #[test]
    fn windows_dir_and_root_metadata_are_hidden() {
        let mut volume = sample();
        volume.upsert(30, ROOT, "Windows", DIR);
        volume.upsert(31, 30, "notepad.exe", FILE);
        volume.set_windows_dir(Some(30));
        volume.upsert(32, ROOT, "$Extend", DIR);
        volume.upsert(33, ROOT, "System Volume Information", DIR);
        assert!(!volume.lineage(31, None).unwrap().visible);
        assert!(!volume.lineage(32, None).unwrap().visible);
        assert!(!volume.lineage(33, None).unwrap().visible);
    }

    #[test]
    fn root_attributes_do_not_hide_everything() {
        let mut volume = sample();
        volume.upsert(
            ROOT,
            ROOT,
            ".",
            Attributes {
                dir: true,
                hidden: true,
                system: true,
            },
        );
        assert!(volume.lineage(22, None).unwrap().visible);
    }

    #[test]
    fn lineage_is_none_for_orphans() {
        let mut volume = sample();
        volume.upsert(30, 99, "orphan.txt", FILE);
        assert!(volume.lineage(30, None).is_none());
    }
}
