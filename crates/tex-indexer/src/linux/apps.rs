use std::ffi::CStr;
use std::fs;
use std::path::Path;

use tex_index::DesktopApp;

use crate::desktop::desktop_id;

const MAX_DEPTH: usize = 3;

/// The apps of the `.desktop` files in `dirs` and their subfolders, `dirs` in order.
pub fn read_apps(dirs: &[String]) -> Vec<DesktopApp> {
    let mut apps = Vec::new();
    for dir in dirs {
        collect(Path::new(dir), "", &mut apps, 0);
    }
    apps
}

fn collect(folder: &Path, prefix: &str, apps: &mut Vec<DesktopApp>, depth: usize) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        let relative = format!("{prefix}{name}");
        if path.is_dir() {
            if depth < MAX_DEPTH {
                collect(&path, &format!("{relative}/"), apps, depth + 1);
            }
            continue;
        }
        if !name.ends_with(".desktop") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(app) = DesktopApp::parse(&desktop_id(&relative), &path.to_string_lossy(), &text)
        {
            apps.push(app);
        }
    }
}

/// The home folder of `uid` in the user database (`getpwuid_r`).
pub fn home_of(uid: u32) -> Option<String> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer: Vec<libc::c_char> = vec![0; 16 * 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    let home = unsafe { CStr::from_ptr(entry.pw_dir) }.to_str().ok()?;
    (!home.is_empty()).then(|| home.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tex-apps-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(name: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n")
    }

    #[test]
    fn reads_desktop_files_with_their_ids() {
        let first = temp("first");
        let second = temp("second");
        fs::create_dir(first.join("kde4")).unwrap();
        fs::write(first.join("kde4/okular.desktop"), entry("Okular")).unwrap();
        fs::write(first.join("tex.desktop"), entry("Tex")).unwrap();
        fs::write(first.join("notes.txt"), "x").unwrap();
        fs::write(second.join("tex.desktop"), entry("Tex Old")).unwrap();
        let dirs = [
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
            "/definitely/not/here".to_owned(),
        ];
        let apps = read_apps(&dirs);
        let ids: Vec<&str> = apps.iter().map(|app| app.id.as_str()).collect();
        assert_eq!(ids, ["kde4-okular.desktop", "tex.desktop", "tex.desktop"]);
        assert_eq!(apps[1].name(""), "Tex");
        assert_eq!(apps[2].name(""), "Tex Old");
        assert!(apps[0].path.ends_with("kde4/okular.desktop"));
        fs::remove_dir_all(&first).unwrap();
        fs::remove_dir_all(&second).unwrap();
    }

    #[test]
    fn finds_the_home_of_the_current_user() {
        let uid = unsafe { libc::getuid() };
        let home = home_of(uid).unwrap();
        assert!(home.starts_with('/'));
        assert!(Path::new(&home).is_dir());
        assert_eq!(home_of(u32::MAX - 7), None);
    }
}
