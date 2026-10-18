use std::collections::HashSet;
use std::path::PathBuf;

/// Local file systems on block devices. Pseudo, memory and network file systems (`proc`,
/// `tmpfs`, `overlay`, `squashfs`, `nfs`, `cifs`, `9p`, `fuse.*`) are not here.
const INDEXED: [&str; 17] = [
    "bcachefs", "btrfs", "exfat", "ext2", "ext3", "ext4", "f2fs", "fuseblk", "hfsplus", "jfs",
    "nilfs2", "ntfs", "ntfs3", "reiserfs", "vfat", "xfs", "zfs",
];

/// One line of `/proc/self/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// `major:minor` of the file system.
    pub device: String,
    /// The folder of the file system shown at `point`: `/`, or a subfolder for bind mounts and
    /// btrfs subvolumes.
    pub root: String,
    pub point: String,
    pub fs_type: String,
}

/// Reads every line of `/proc/self/mountinfo`, skipping lines it does not understand.
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Mount> {
    let (left, right) = line.split_once(" - ")?;
    let fields: Vec<&str> = left.split(' ').collect();
    Some(Mount {
        device: (*fields.get(2)?).to_owned(),
        root: unescape(fields.get(3)?),
        point: unescape(fields.get(4)?),
        fs_type: right.split(' ').next()?.to_owned(),
    })
}

/// Undoes the octal escapes of mountinfo (`\040` is a space).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some(code) = bytes.get(i + 1..i + 4)
            && code.iter().all(|digit| (b'0'..=b'7').contains(digit))
            && let Ok(value) = u8::from_str_radix(std::str::from_utf8(code).unwrap_or("x"), 8)
        {
            out.push(value);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mounts to index, sorted by mount point: local file systems, each folder of a file system
/// once. A mount showing a folder already covered by another mount of the same device (a bind
/// mount, the same file system mounted twice) is left out; the one kept has the shallowest root,
/// then the shortest mount point.
pub fn indexable(mounts: &[Mount]) -> Vec<Mount> {
    let mut candidates: Vec<&Mount> = mounts
        .iter()
        .filter(|mount| INDEXED.contains(&mount.fs_type.as_str()) && mount.point.starts_with('/'))
        .collect();
    candidates.sort_by_key(|mount| (mount.root.len(), mount.point.len()));
    let mut chosen: Vec<Mount> = Vec::new();
    for mount in candidates {
        let covered = chosen
            .iter()
            .any(|kept| kept.device == mount.device && inside(&mount.root, &kept.root));
        if !covered {
            chosen.push(mount.clone());
        }
    }
    chosen.sort_by(|a, b| a.point.cmp(&b.point));
    chosen
}

/// Whether `path` is `folder` or below it.
fn inside(path: &str, folder: &str) -> bool {
    folder == "/"
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Every mount point but `point`: the scan of `point` lists them but does not enter them.
pub fn boundaries(mounts: &[Mount], point: &str) -> HashSet<PathBuf> {
    mounts
        .iter()
        .filter(|mount| mount.point != point)
        .map(|mount| PathBuf::from(&mount.point))
        .collect()
}

/// The id of a volume: the file system id, mixed with the shown folder when that is not the
/// root of the file system, so two folders of one file system get different snapshots.
pub fn volume_key(fsid: u64, root: &str) -> u64 {
    if root == "/" {
        fsid
    } else {
        fsid ^ fnv1a(root)
    }
}

/// 64-bit FNV-1a, stable across Rust versions (snapshot names depend on it).
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WSL: &str = "\
81 88 0:31 / /usr/lib/modules/6.18 rw,nosuid - overlay none rw,lowerdir=/modules
82 88 8:16 /6.18/linux-headers /usr/src/linux-headers-6.18 ro,relatime - ext4 /dev/sdb ro
83 88 8:16 /6.18/perf /usr/lib/linux-tools/6.18 ro,relatime - ext4 /dev/sdb ro
88 71 8:48 / / rw,relatime - ext4 /dev/sdd rw,discard
90 89 8:48 / /mnt/wslg/distro ro,relatime shared:3 - ext4 /dev/sdd rw
120 88 0:61 / /proc rw,nosuid shared:16 - proc proc rw
122 88 0:63 / /run rw,nosuid,nodev shared:18 - tmpfs none rw,mode=755
140 88 0:72 / /mnt/d rw,noatime - 9p D:\\134 rw,aname=drvfs
";

    const DESKTOP: &str = "\
59 1 0:28 /root / rw,relatime shared:1 - btrfs /dev/vda3 rw,subvol=/root
60 59 0:28 /home /home rw,relatime shared:2 - btrfs /dev/vda3 rw,subvol=/home
61 59 0:28 /home/ana/Projetos /srv/projetos rw,relatime - btrfs /dev/vda3 rw,subvol=/home
62 59 252:1 / /boot/efi rw,relatime shared:3 - vfat /dev/vda1 rw
63 59 8:17 / /media/ana/My\\040Disk rw,nosuid shared:4 - fuseblk /dev/sdb1 rw
64 59 0:45 / /home/ana/.cache/doc rw - fuse.portal portal rw
65 59 7:3 / /snap/core/1 ro shared:5 - squashfs /dev/loop3 ro
66 59 0:50 / /mnt/nas rw - nfs4 nas:/data rw
";

    fn points(mounts: &[Mount]) -> Vec<&str> {
        mounts.iter().map(|mount| mount.point.as_str()).collect()
    }

    #[test]
    fn parses_mountinfo_lines() {
        let mounts = parse_mountinfo(DESKTOP);
        assert_eq!(mounts.len(), 8);
        assert_eq!(
            mounts[0],
            Mount {
                device: "0:28".into(),
                root: "/root".into(),
                point: "/".into(),
                fs_type: "btrfs".into(),
            }
        );
        assert_eq!(mounts[4].point, "/media/ana/My Disk");
        assert_eq!(mounts[4].fs_type, "fuseblk");
        assert!(parse_mountinfo("garbage\n\n1 2 3\n").is_empty());
    }

    #[test]
    fn indexes_local_file_systems_once() {
        assert_eq!(
            points(&indexable(&parse_mountinfo(DESKTOP))),
            ["/", "/boot/efi", "/home", "/media/ana/My Disk"]
        );
        assert_eq!(
            points(&indexable(&parse_mountinfo(WSL))),
            [
                "/",
                "/usr/lib/linux-tools/6.18",
                "/usr/src/linux-headers-6.18"
            ]
        );
    }

    #[test]
    fn scans_stop_at_every_other_mount_point() {
        let mounts = parse_mountinfo(WSL);
        let stops = boundaries(&mounts, "/");
        assert!(stops.contains(&PathBuf::from("/proc")));
        assert!(stops.contains(&PathBuf::from("/mnt/d")));
        assert!(stops.contains(&PathBuf::from("/mnt/wslg/distro")));
        assert!(!stops.contains(&PathBuf::from("/")));
        assert_eq!(stops.len(), mounts.len() - 1);
    }

    #[test]
    fn volume_keys_tell_folders_of_one_file_system_apart() {
        assert_eq!(volume_key(0x1234, "/"), 0x1234);
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(volume_key(7, "/root"), volume_key(7, "/home"));
        assert_eq!(volume_key(7, "/home"), volume_key(7, "/home"));
    }
}
