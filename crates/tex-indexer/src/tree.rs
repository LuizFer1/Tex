use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tex_index::{Attributes, DenseIds, Owner, Record, VolumeIndex};

/// What the disk says about one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub dir: bool,
    /// Owner and mode, for directories.
    pub owner: Option<Owner>,
    /// `name_to_handle_at` key (handle type, then handle bytes), for directories.
    pub handle: Option<Vec<u8>>,
}

impl Node {
    pub fn file() -> Self {
        Self {
            dir: false,
            owner: None,
            handle: None,
        }
    }
}

/// Adds `name` inside `parent`, or updates it when it is there. Returns its record.
pub fn add(
    index: &mut VolumeIndex,
    ids: &mut DenseIds,
    parent: Record,
    name: &str,
    node: &Node,
) -> Record {
    let attributes = Attributes {
        dir: node.dir,
        ..Attributes::default()
    };
    let record = ids.upsert(index, parent, name, attributes);
    index.set_owner(record, node.owner);
    if let Some(handle) = &node.handle {
        ids.set_dir(handle, record);
    }
    record
}

/// Adds everything below `top` (the folder at `path`), depth first. `list` reads one folder;
/// folders in `boundaries` (other mount points) are added but not entered. `progress` gets the
/// number of entries added so far after each folder. Returns that number.
pub fn walk(
    index: &mut VolumeIndex,
    ids: &mut DenseIds,
    top: Record,
    path: &Path,
    boundaries: &HashSet<PathBuf>,
    list: &mut dyn FnMut(&Path) -> Vec<(String, Node)>,
    progress: &mut dyn FnMut(usize),
) -> usize {
    let mut added = 0;
    let mut pending = vec![(top, path.to_path_buf())];
    while let Some((record, folder)) = pending.pop() {
        for (name, mut node) in list(&folder) {
            let child_path = folder.join(&name);
            let crossing = boundaries.contains(&child_path);
            if crossing {
                node.handle = None;
            }
            let child = add(index, ids, record, &name, &node);
            added += 1;
            if node.dir && !crossing {
                pending.push((child, child_path));
            }
        }
        progress(added);
    }
    added
}

/// Brings the entry `name` inside `parent` in line with the disk, where `found` is what is
/// there now (`None` when nothing is). A folder already indexed under another name (same
/// handle) is moved with its subtree. Returns the record of a folder new to the index, which
/// the caller must walk.
pub fn reconcile(
    index: &mut VolumeIndex,
    ids: &mut DenseIds,
    parent: Record,
    name: &str,
    found: Option<Node>,
) -> Option<Record> {
    let Some(node) = found else {
        ids.remove(index, parent, name);
        return None;
    };
    let existing = ids.child(index, parent, name);
    if let Some(record) = existing
        && index.is_dir(record)
        && !node.dir
    {
        ids.remove(index, parent, name);
    }
    if node.dir
        && let Some(handle) = &node.handle
        && let Some(moved) = ids.dir(handle)
        && Some(moved) != existing
        && let (Some(old_parent), Some(old_name)) =
            (index.parent(moved), index.name(moved).map(str::to_owned))
    {
        ids.rename(index, (old_parent, &old_name), (parent, name));
        index.set_owner(moved, node.owner);
        return None;
    }
    let was_dir = ids
        .child(index, parent, name)
        .is_some_and(|record| index.is_dir(record));
    let record = add(index, ids, parent, name, &node);
    (node.dir && !was_dir).then_some(record)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use tex_index::ROOT;

    fn at(parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(PathBuf::from("/"), |path, part| path.join(part))
    }

    fn folder(owner_uid: u32, handle: &[u8]) -> Node {
        Node {
            dir: true,
            owner: Some(Owner {
                uid: owner_uid,
                gid: owner_uid,
                mode: 0o755,
            }),
            handle: Some(handle.to_vec()),
        }
    }

    fn disk() -> HashMap<PathBuf, Vec<(String, Node)>> {
        HashMap::from([
            (
                at(&[]),
                vec![
                    ("home".into(), folder(0, b"home")),
                    ("boot".into(), folder(0, b"boot")),
                    ("notes.txt".into(), Node::file()),
                ],
            ),
            (
                at(&["home"]),
                vec![
                    ("ana".into(), folder(1000, b"ana")),
                    ("readme".into(), Node::file()),
                ],
            ),
            (
                at(&["home", "ana"]),
                vec![("plano.txt".into(), Node::file())],
            ),
            (at(&["boot"]), vec![("vmlinuz".into(), Node::file())]),
        ])
    }

    fn scan(boundaries: &HashSet<PathBuf>) -> (VolumeIndex, DenseIds, Vec<usize>) {
        let disk = disk();
        let mut index = VolumeIndex::new("/");
        let mut ids = DenseIds::new();
        let mut steps = Vec::new();
        walk(
            &mut index,
            &mut ids,
            ROOT,
            &at(&[]),
            boundaries,
            &mut |path| disk.get(path).cloned().unwrap_or_default(),
            &mut |added| steps.push(added),
        );
        (index, ids, steps)
    }

    #[test]
    fn walk_indexes_the_tree_with_owners_and_handles() {
        let (index, ids, steps) = scan(&HashSet::new());
        assert_eq!(index.len(), 7);
        assert_eq!(steps.len(), 4);
        assert_eq!(steps.last(), Some(&7));
        let ana = index.lookup("/home/ana").unwrap();
        assert_eq!(ids.dir(b"ana"), Some(ana));
        assert_eq!(index.owner(ana).map(|owner| owner.uid), Some(1000));
        assert!(index.lookup("/home/ana/plano.txt").is_some());
        assert_eq!(index.owner(index.lookup("/notes.txt").unwrap()), None);
    }

    #[test]
    fn walk_stops_at_other_mount_points() {
        let (index, ids, _) = scan(&HashSet::from([at(&["boot"])]));
        let boot = index.lookup("/boot").unwrap();
        assert!(index.is_dir(boot));
        assert!(index.lookup("/boot/vmlinuz").is_none());
        assert_eq!(ids.dir(b"boot"), None);
        assert_eq!(index.len(), 6);
    }

    #[test]
    fn reconcile_removes_what_is_gone_with_its_subtree() {
        let (mut index, mut ids, _) = scan(&HashSet::new());
        let home = index.lookup("/home").unwrap();
        assert_eq!(reconcile(&mut index, &mut ids, ROOT, "home", None), None);
        assert!(index.lookup("/home/ana/plano.txt").is_none());
        assert_eq!(index.name(home), None);
        assert_eq!(index.len(), 3);
        assert_eq!(reconcile(&mut index, &mut ids, ROOT, "missing", None), None);
    }

    #[test]
    fn reconcile_adds_files_and_asks_to_walk_new_folders() {
        let (mut index, mut ids, _) = scan(&HashSet::new());
        let ana = index.lookup("/home/ana").unwrap();
        assert_eq!(
            reconcile(&mut index, &mut ids, ana, "novo.txt", Some(Node::file())),
            None
        );
        assert!(index.lookup("/home/ana/novo.txt").is_some());
        let created = reconcile(
            &mut index,
            &mut ids,
            ana,
            "fotos",
            Some(folder(1000, b"fotos")),
        );
        assert_eq!(created, index.lookup("/home/ana/fotos"));
        assert_eq!(ids.dir(b"fotos"), created);
        assert_eq!(
            reconcile(
                &mut index,
                &mut ids,
                ana,
                "fotos",
                Some(folder(1000, b"fotos"))
            ),
            None
        );
    }

    #[test]
    fn reconcile_moves_a_known_folder_with_its_subtree() {
        let (mut index, mut ids, _) = scan(&HashSet::new());
        let ana = index.lookup("/home/ana").unwrap();
        let home = index.lookup("/home").unwrap();
        let mut moved = folder(1000, b"ana");
        moved.owner = Some(Owner {
            uid: 1000,
            gid: 1000,
            mode: 0o700,
        });
        assert_eq!(
            reconcile(&mut index, &mut ids, ROOT, "ana-old", Some(moved)),
            None
        );
        assert_eq!(index.lookup("/ana-old"), Some(ana));
        assert!(index.lookup("/ana-old/plano.txt").is_some());
        assert_eq!(index.owner(ana).map(|owner| owner.mode), Some(0o700));
        assert_eq!(reconcile(&mut index, &mut ids, home, "ana", None), None);
        assert!(index.lookup("/ana-old/plano.txt").is_some());
    }

    #[test]
    fn reconcile_replaces_a_folder_by_a_file_and_back() {
        let (mut index, mut ids, _) = scan(&HashSet::new());
        let home = index.lookup("/home").unwrap();
        assert_eq!(
            reconcile(&mut index, &mut ids, home, "ana", Some(Node::file())),
            None
        );
        let ana = index.lookup("/home/ana").unwrap();
        assert!(!index.is_dir(ana));
        assert!(index.lookup("/home/ana/plano.txt").is_none());
        assert_eq!(
            reconcile(
                &mut index,
                &mut ids,
                home,
                "ana",
                Some(folder(1000, b"ana-2"))
            ),
            Some(ana)
        );
        assert!(index.is_dir(ana));
    }
}
