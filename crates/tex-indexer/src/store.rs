use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use tex_index::{SnapshotError, SnapshotHeader, VolumeIndex, read_snapshot, write_snapshot};

pub fn snapshot_path(dir: &Path, serial: u64) -> PathBuf {
    dir.join(format!("{serial:08X}.bin"))
}

/// Writes to a temporary file and renames it, so a crash never leaves a half-written snapshot.
pub fn save(dir: &Path, index: &VolumeIndex, header: &SnapshotHeader) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = snapshot_path(dir, header.serial);
    let temporary = path.with_extension("tmp");
    let mut out = BufWriter::new(File::create(&temporary)?);
    write_snapshot(&mut out, index, header)?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    fs::rename(&temporary, &path)
}

pub fn load(dir: &Path, serial: u64) -> Result<(VolumeIndex, SnapshotHeader), SnapshotError> {
    let file = File::open(snapshot_path(dir, serial))?;
    read_snapshot(&mut BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_index::{Attributes, ROOT, SnapshotError};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tex-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_then_load() {
        let dir = temp_dir("roundtrip");
        let mut index = VolumeIndex::new("C:");
        index.upsert(20, ROOT, "notes.txt", Attributes::default());
        let header = SnapshotHeader {
            serial: 0xABCD_0001,
            journal_id: 3,
            next_usn: 77,
        };
        save(&dir, &index, &header).unwrap();
        let (loaded, loaded_header) = load(&dir, 0xABCD_0001).unwrap();
        assert_eq!(loaded_header, header);
        assert_eq!(loaded.path(20).as_deref(), Some(r"C:\notes.txt"));
        assert!(
            !snapshot_path(&dir, 0xABCD_0001)
                .with_extension("tmp")
                .exists()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_snapshot_is_not_found() {
        let dir = temp_dir("missing");
        match load(&dir, 1) {
            Err(SnapshotError::Io(error)) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
            _ => panic!("expected NotFound"),
        }
    }

    #[test]
    fn file_name_is_the_serial_in_hex() {
        assert_eq!(
            snapshot_path(Path::new("x"), 0x00AB_CDEF),
            Path::new("x").join("00ABCDEF.bin")
        );
    }

    #[test]
    fn wide_serials_keep_every_digit() {
        assert_eq!(
            snapshot_path(Path::new("x"), 0x1234_5678_9ABC_DEF0),
            Path::new("x").join("123456789ABCDEF0.bin")
        );
    }
}
