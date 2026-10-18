use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null;

use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
};

/// Writes `bytes` beside `path` and swaps the copy in, so the file is never left half written.
/// An existing file keeps its attributes, ACL and creation time; a missing one is created.
pub fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temp_path(path);
    let result = write_and_swap(path, &temp, bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tex-tmp");
    path.with_file_name(name)
}

fn write_and_swap(path: &Path, temp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let target = wide(path.as_os_str());
    let replacement = wide(temp.as_os_str());
    let replaced = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            null(),
            null(),
        )
    };
    if replaced != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if path.exists() {
        return Err(error);
    }
    let moved = unsafe {
        MoveFileExW(
            replacement.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> Temp {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tex-save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tex-tmp"))
            .collect()
    }

    #[test]
    fn replaces_the_content() {
        let dir = temp_dir();
        let path = dir.0.join("a.txt");
        std::fs::write(&path, b"old").unwrap();
        replace_file(&path, b"new content").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new content");
        assert!(leftovers(&dir.0).is_empty());
    }

    #[test]
    fn creates_a_file_that_is_gone() {
        let dir = temp_dir();
        let path = dir.0.join("gone.md");
        replace_file(&path, b"back").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"back");
        assert!(leftovers(&dir.0).is_empty());
    }

    #[test]
    fn a_missing_folder_fails_cleanly() {
        let dir = temp_dir();
        let path = dir.0.join("no").join("such.txt");
        assert!(replace_file(&path, b"x").is_err());
        assert!(leftovers(&dir.0).is_empty());
    }

    #[test]
    fn a_read_only_file_is_not_replaced() {
        let dir = temp_dir();
        let path = dir.0.join("ro.txt");
        std::fs::write(&path, b"keep").unwrap();
        let writable = std::fs::metadata(&path).unwrap().permissions();
        let mut permissions = writable.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(replace_file(&path, b"changed").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        assert!(leftovers(&dir.0).is_empty());
        std::fs::set_permissions(&path, writable).unwrap();
    }
}
