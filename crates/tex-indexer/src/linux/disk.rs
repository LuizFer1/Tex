use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use tex_index::{DenseIds, Owner, ROOT, VolumeIndex};

use crate::tree::{self, Node};

const MAX_HANDLE_BYTES: usize = 128;

#[repr(C)]
struct HandleBuffer {
    bytes: u32,
    kind: i32,
    data: [u8; MAX_HANDLE_BYTES],
}

/// The entries of `folder`, with owner and handle for the folders among them. Names that are
/// not UTF-8 are skipped, and a folder that can not be read lists nothing.
pub fn list(folder: &Path) -> Vec<(String, Node)> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let node = if entry.file_type().ok()?.is_dir() {
                probe(&entry.path())?
            } else {
                Node::file()
            };
            Some((name, node))
        })
        .collect()
}

/// What is at `path` now, without following a final symlink. `None` when nothing is.
pub fn probe(path: &Path) -> Option<Node> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() {
        return Some(Node::file());
    }
    Some(Node {
        dir: true,
        owner: Some(Owner {
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: (metadata.mode() & 0o7777) as u16,
        }),
        handle: handle(path),
    })
}

/// The `name_to_handle_at` key of `path`: handle type, then handle bytes, as fanotify reports
/// the folders of its events. `None` where the file system has no handles.
pub fn handle(path: &Path) -> Option<Vec<u8>> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buffer = HandleBuffer {
        bytes: MAX_HANDLE_BYTES as u32,
        kind: 0,
        data: [0; MAX_HANDLE_BYTES],
    };
    let mut mount_id: libc::c_int = 0;
    let result = unsafe {
        libc::name_to_handle_at(
            libc::AT_FDCWD,
            path.as_ptr(),
            (&raw mut buffer).cast(),
            &mut mount_id,
            0,
        )
    };
    if result != 0 {
        return None;
    }
    let mut key = buffer.kind.to_ne_bytes().to_vec();
    key.extend_from_slice(buffer.data.get(..buffer.bytes as usize)?);
    Some(key)
}

/// The file system id (`f_fsid` of `statfs`) at `path` and how many inodes it uses.
pub fn file_system(path: &Path) -> io::Result<(u64, u64)> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut info: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut info) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let halves = unsafe { std::mem::transmute::<libc::fsid_t, [u32; 2]>(info.f_fsid) };
    let fsid = u64::from(halves[0]) | (u64::from(halves[1]) << 32);
    let used: u64 = info.f_files.saturating_sub(info.f_ffree);
    Ok((fsid, used))
}

/// Indexes the tree at `point` (a mount point), without entering `boundaries`.
pub fn scan(
    point: &str,
    boundaries: &HashSet<PathBuf>,
    progress: &mut dyn FnMut(usize),
) -> (VolumeIndex, DenseIds) {
    let mut index = VolumeIndex::new(point);
    let mut ids = DenseIds::new();
    let path = Path::new(point);
    if let Some(node) = probe(path) {
        index.set_owner(ROOT, node.owner);
        if let Some(handle) = &node.handle {
            ids.set_dir(handle, ROOT);
        }
    }
    tree::walk(
        &mut index, &mut ids, ROOT, path, boundaries, &mut list, progress,
    );
    (index, ids)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tex-disk-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_folders_with_owner_and_handle() {
        let root = temp("list");
        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join("notes.txt"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("docs"), root.join("link")).unwrap();
        let mut entries = list(&root);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["docs", "link", "notes.txt"]);
        let docs = &entries[0].1;
        assert!(docs.dir);
        assert_eq!(docs.owner.unwrap().uid, unsafe { libc::getuid() });
        assert!(docs.handle.is_some());
        assert_eq!(entries[1].1, Node::file());
        assert_eq!(entries[2].1, Node::file());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn probe_reads_the_mode_and_misses_what_is_gone() {
        let root = temp("probe");
        let folder = root.join("private");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(probe(&folder).unwrap().owner.unwrap().mode, 0o750);
        assert_eq!(probe(&root.join("missing")), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn handles_follow_a_folder_across_renames() {
        let root = temp("rename");
        fs::create_dir(root.join("a")).unwrap();
        let before = handle(&root.join("a")).unwrap();
        fs::rename(root.join("a"), root.join("b")).unwrap();
        assert_eq!(handle(&root.join("b")), Some(before));
        assert_eq!(handle(&root.join("a")), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scans_a_real_tree() {
        let root = temp("scan");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/c.txt"), "x").unwrap();
        fs::write(root.join(".hidden"), "x").unwrap();
        let point = root.to_str().unwrap();
        let (index, ids) = scan(point, &HashSet::new(), &mut |_| {});
        assert_eq!(index.len(), 4);
        let a = index.lookup(&format!("{point}/a")).unwrap();
        assert_eq!(ids.dir(&handle(&root.join("a")).unwrap()), Some(a));
        assert_eq!(ids.dir(&handle(&root).unwrap()), Some(ROOT));
        assert!(index.owner(ROOT).is_some());
        assert!(index.lookup(&format!("{point}/a/b/c.txt")).is_some());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn file_systems_have_an_id() {
        let (fsid, used) = file_system(Path::new("/")).unwrap();
        assert_ne!(fsid, 0);
        assert!(used > 0);
        assert!(file_system(Path::new("/definitely/not/here")).is_err());
    }
}
