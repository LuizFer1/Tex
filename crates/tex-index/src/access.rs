use crate::entry::{ROOT, Record};
use crate::volume::VolumeIndex;

const MAX_DEPTH: usize = 512;
const SEARCH: u8 = 1;
const READ: u8 = 2;

/// Owner and permission bits of a directory on a Unix volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    /// The permission bits of `st_mode`, such as `0o755`.
    pub mode: u16,
}

/// Who is searching: the user id and every group of that user, the primary one included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub uid: u32,
    pub groups: Vec<u32>,
}

impl Viewer {
    /// `SEARCH` (x) and `READ` (r) as `owner` grants them to this viewer: the owner's bits for
    /// its owner, the group's for a member, the others' bits for everyone else.
    fn rights(&self, owner: &Owner) -> u8 {
        let shift = if self.uid == owner.uid {
            6
        } else if self.groups.contains(&owner.gid) {
            3
        } else {
            0
        };
        let bits = (owner.mode >> shift) & 0o7;
        let mut rights = 0;
        if bits & 0o1 != 0 {
            rights |= SEARCH;
        }
        if bits & 0o4 != 0 {
            rights |= READ;
        }
        rights
    }
}

/// One byte per record of `volume` with the rights `viewer` has on that directory. Records
/// without an owner (files, folders the scan could not read) grant nothing.
pub(crate) fn rights_table(volume: &VolumeIndex, viewer: &Viewer) -> Vec<u8> {
    let mut table = vec![0; volume.entries().len().max(ROOT as usize + 1)];
    for (&record, owner) in volume.owners() {
        if let Some(slot) = table.get_mut(record as usize) {
            *slot = viewer.rights(owner);
        }
    }
    table
}

/// Whether `record` can be reached with `table`: read and search on its folder and search on
/// every folder above it, up to the volume's root.
pub(crate) fn reachable(volume: &VolumeIndex, record: Record, table: &[u8]) -> bool {
    let allows = |record: Record, needed: u8| {
        table
            .get(record as usize)
            .is_some_and(|rights| rights & needed == needed)
    };
    let Some(mut current) = volume.parent(record) else {
        return false;
    };
    if !allows(current, SEARCH | READ) {
        return false;
    }
    for _ in 0..MAX_DEPTH {
        if current == ROOT {
            return true;
        }
        let Some(parent) = volume.parent(current) else {
            return false;
        };
        if !allows(parent, SEARCH) {
            return false;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rights_follow_owner_group_and_others() {
        let owner = Owner {
            uid: 1000,
            gid: 100,
            mode: 0o751,
        };
        let me = Viewer {
            uid: 1000,
            groups: vec![1000],
        };
        let member = Viewer {
            uid: 1001,
            groups: vec![1001, 100],
        };
        let stranger = Viewer {
            uid: 1002,
            groups: vec![1002],
        };
        assert_eq!(me.rights(&owner), SEARCH | READ);
        assert_eq!(member.rights(&owner), SEARCH | READ);
        assert_eq!(stranger.rights(&owner), SEARCH);
        let closed_to_owner = Owner {
            uid: 1000,
            gid: 100,
            mode: 0o077,
        };
        assert_eq!(me.rights(&closed_to_owner), 0);
        assert_eq!(member.rights(&closed_to_owner), SEARCH | READ);
    }
}
