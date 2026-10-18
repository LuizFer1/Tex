use std::collections::HashMap;
use std::collections::hash_map::Entry as Slot;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::entry::{Attributes, ROOT, Record};
use crate::volume::VolumeIndex;

/// Hands out dense records for file systems without MFT record numbers and finds them again:
/// by directory handle (`name_to_handle_at`) and by parent and name. Directory records are never
/// reused, so entries orphaned by a missed event can not reappear under a new directory.
pub struct DenseIds {
    next: Record,
    free: Vec<Record>,
    hash: fn(Record, &str) -> u64,
    children: HashMap<u64, Record>,
    overflow: HashMap<(Record, String), Record>,
    dirs: HashMap<Box<[u8]>, Record>,
    handles: HashMap<Record, Box<[u8]>>,
}

impl Default for DenseIds {
    fn default() -> Self {
        Self::new()
    }
}

impl DenseIds {
    pub fn new() -> Self {
        Self::with_hash(child_key)
    }

    fn with_hash(hash: fn(Record, &str) -> u64) -> Self {
        Self {
            next: ROOT + 1,
            free: Vec::new(),
            hash,
            children: HashMap::new(),
            overflow: HashMap::new(),
            dirs: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    /// Record of the directory with this file handle.
    pub fn dir(&self, handle: &[u8]) -> Option<Record> {
        self.dirs.get(handle).copied()
    }

    /// Ties a directory's file handle to its record, replacing any older handle of either.
    pub fn set_dir(&mut self, handle: &[u8], record: Record) {
        if let Some(old) = self.handles.insert(record, handle.into()) {
            self.dirs.remove(&old);
        }
        if let Some(previous) = self.dirs.insert(handle.into(), record)
            && previous != record
        {
            self.handles.remove(&previous);
        }
    }

    /// Record of the entry `name` inside `parent`.
    pub fn child(&self, index: &VolumeIndex, parent: Record, name: &str) -> Option<Record> {
        if let Some(&record) = self.children.get(&(self.hash)(parent, name))
            && is_at(index, record, parent, name)
        {
            return Some(record);
        }
        if self.overflow.is_empty() {
            return None;
        }
        self.overflow.get(&(parent, name.to_owned())).copied()
    }

    /// Creates `name` under `parent`, or updates it when it is already there. Returns its record.
    pub fn upsert(
        &mut self,
        index: &mut VolumeIndex,
        parent: Record,
        name: &str,
        attributes: Attributes,
    ) -> Record {
        if let Some(record) = self.child(index, parent, name) {
            index.upsert(record, parent, name, attributes);
            return record;
        }
        let record = self.free.pop().unwrap_or_else(|| {
            let record = self.next;
            self.next += 1;
            record
        });
        index.upsert(record, parent, name, attributes);
        self.remember(parent, name, record);
        record
    }

    /// Moves `from` to `to`, keeping its record and therefore its subtree. An entry already at
    /// `to` is removed first, as the file system replaces it. `None` when `from` is unknown.
    pub fn rename(
        &mut self,
        index: &mut VolumeIndex,
        from: (Record, &str),
        to: (Record, &str),
    ) -> Option<Record> {
        let record = self.child(index, from.0, from.1)?;
        if self
            .child(index, to.0, to.1)
            .is_some_and(|target| target != record)
        {
            self.remove(index, to.0, to.1);
        }
        let entry = *index.entry(record)?;
        self.forget(from.0, from.1, record);
        let attributes = Attributes {
            dir: entry.is_dir(),
            hidden: entry.is_hidden(),
            system: entry.is_system(),
        };
        index.upsert(record, to.0, to.1, attributes);
        self.remember(to.0, to.1, record);
        Some(record)
    }

    /// Removes `name` from `parent`. A file's record goes to the next new entry; a directory's
    /// record is never handed out again.
    pub fn remove(
        &mut self,
        index: &mut VolumeIndex,
        parent: Record,
        name: &str,
    ) -> Option<Record> {
        let record = self.child(index, parent, name)?;
        let is_dir = index.entry(record).is_some_and(|entry| entry.is_dir());
        self.forget(parent, name, record);
        index.remove(record);
        if let Some(handle) = self.handles.remove(&record) {
            self.dirs.remove(&handle);
        }
        if !is_dir {
            self.free.push(record);
        }
        Some(record)
    }

    fn remember(&mut self, parent: Record, name: &str, record: Record) {
        match self.children.entry((self.hash)(parent, name)) {
            Slot::Vacant(slot) => {
                slot.insert(record);
            }
            Slot::Occupied(slot) if *slot.get() == record => {}
            Slot::Occupied(_) => {
                self.overflow.insert((parent, name.to_owned()), record);
            }
        }
    }

    fn forget(&mut self, parent: Record, name: &str, record: Record) {
        let key = (self.hash)(parent, name);
        if self.children.get(&key) == Some(&record) {
            self.children.remove(&key);
        } else {
            self.overflow.remove(&(parent, name.to_owned()));
        }
    }
}

fn child_key(parent: Record, name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    parent.hash(&mut hasher);
    name.hash(&mut hasher);
    hasher.finish()
}

fn is_at(index: &VolumeIndex, record: Record, parent: Record, name: &str) -> bool {
    index
        .entry(record)
        .is_some_and(|entry| entry.parent == parent && index.entry_name(entry) == name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: Attributes = Attributes {
        dir: false,
        hidden: false,
        system: false,
    };
    const DIR: Attributes = Attributes {
        dir: true,
        hidden: false,
        system: false,
    };

    fn setup() -> (VolumeIndex, DenseIds) {
        (VolumeIndex::new("/"), DenseIds::new())
    }

    #[test]
    fn records_are_handed_out_in_order() {
        let (mut index, mut ids) = setup();
        let home = ids.upsert(&mut index, ROOT, "home", DIR);
        let luis = ids.upsert(&mut index, home, "luis", DIR);
        let notes = ids.upsert(&mut index, luis, "notes.txt", FILE);
        assert_eq!([home, luis, notes], [ROOT + 1, ROOT + 2, ROOT + 3]);
        assert_eq!(index.path(notes).as_deref(), Some("/home/luis/notes.txt"));
        assert_eq!(ids.child(&index, luis, "notes.txt"), Some(notes));
        assert_eq!(ids.child(&index, luis, "other.txt"), None);
        assert_eq!(ids.child(&index, home, "notes.txt"), None);
    }

    #[test]
    fn upsert_of_an_existing_name_keeps_the_record() {
        let (mut index, mut ids) = setup();
        let first = ids.upsert(&mut index, ROOT, "a.txt", FILE);
        let again = ids.upsert(&mut index, ROOT, "a.txt", FILE);
        assert_eq!(first, again);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn directory_handles_map_to_records() {
        let (mut index, mut ids) = setup();
        ids.set_dir(b"root", ROOT);
        let home = ids.upsert(&mut index, ROOT, "home", DIR);
        ids.set_dir(b"home-1", home);
        assert_eq!(ids.dir(b"root"), Some(ROOT));
        assert_eq!(ids.dir(b"home-1"), Some(home));
        ids.set_dir(b"home-2", home);
        assert_eq!(ids.dir(b"home-1"), None);
        assert_eq!(ids.dir(b"home-2"), Some(home));
        assert_eq!(ids.dir(b"missing"), None);
    }

    #[test]
    fn rename_moves_the_subtree() {
        let (mut index, mut ids) = setup();
        let a = ids.upsert(&mut index, ROOT, "a", DIR);
        let b = ids.upsert(&mut index, ROOT, "b", DIR);
        let docs = ids.upsert(&mut index, a, "docs", DIR);
        let file = ids.upsert(&mut index, docs, "x.txt", FILE);
        assert_eq!(
            ids.rename(&mut index, (a, "docs"), (b, "papers")),
            Some(docs)
        );
        assert_eq!(index.path(file).as_deref(), Some("/b/papers/x.txt"));
        assert_eq!(ids.child(&index, a, "docs"), None);
        assert_eq!(ids.child(&index, b, "papers"), Some(docs));
        assert!(index.entry(docs).unwrap().is_dir());
        assert_eq!(ids.rename(&mut index, (a, "missing"), (b, "x")), None);
    }

    #[test]
    fn rename_replaces_the_destination() {
        let (mut index, mut ids) = setup();
        let old = ids.upsert(&mut index, ROOT, "old.txt", FILE);
        let new = ids.upsert(&mut index, ROOT, "new.txt", FILE);
        assert_eq!(
            ids.rename(&mut index, (ROOT, "new.txt"), (ROOT, "old.txt")),
            Some(new)
        );
        assert_eq!(index.len(), 1);
        assert_eq!(index.name(old), None);
        assert_eq!(ids.child(&index, ROOT, "old.txt"), Some(new));
        assert_eq!(ids.child(&index, ROOT, "new.txt"), None);
    }

    #[test]
    fn removed_files_free_their_record_but_directories_do_not() {
        let (mut index, mut ids) = setup();
        let dir = ids.upsert(&mut index, ROOT, "dir", DIR);
        ids.set_dir(b"dir", dir);
        let file = ids.upsert(&mut index, dir, "f.txt", FILE);
        assert_eq!(ids.remove(&mut index, dir, "f.txt"), Some(file));
        assert_eq!(ids.remove(&mut index, ROOT, "dir"), Some(dir));
        assert_eq!(ids.dir(b"dir"), None);
        assert_eq!(index.len(), 0);
        assert_eq!(ids.upsert(&mut index, ROOT, "next.txt", FILE), file);
        assert_eq!(ids.upsert(&mut index, ROOT, "later.txt", FILE), dir + 2);
        assert_eq!(ids.remove(&mut index, ROOT, "missing"), None);
    }

    #[test]
    fn colliding_keys_still_find_every_entry() {
        let mut index = VolumeIndex::new("/");
        let mut ids = DenseIds::with_hash(|_, _| 7);
        let a = ids.upsert(&mut index, ROOT, "a", DIR);
        let b = ids.upsert(&mut index, ROOT, "b", FILE);
        let c = ids.upsert(&mut index, a, "c", FILE);
        assert_eq!(ids.child(&index, ROOT, "a"), Some(a));
        assert_eq!(ids.child(&index, ROOT, "b"), Some(b));
        assert_eq!(ids.child(&index, a, "c"), Some(c));
        assert_eq!(ids.remove(&mut index, ROOT, "a"), Some(a));
        assert_eq!(ids.child(&index, ROOT, "b"), Some(b));
        assert_eq!(ids.child(&index, a, "c"), Some(c));
        assert_eq!(ids.rename(&mut index, (ROOT, "b"), (ROOT, "d")), Some(b));
        assert_eq!(ids.child(&index, ROOT, "d"), Some(b));
        assert_eq!(ids.child(&index, ROOT, "b"), None);
    }
}
