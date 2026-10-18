/// Folders with the `.desktop` files every user sees, the most important first.
pub const SYSTEM_APP_DIRS: [&str; 4] = [
    "/usr/local/share/applications",
    "/usr/share/applications",
    "/var/lib/flatpak/exports/share/applications",
    "/var/lib/snapd/desktop/applications",
];

/// The folders with the apps of the user whose home is `home`, the most important first.
pub fn user_app_dirs(home: &str) -> [String; 2] {
    let home = home.trim_end_matches('/');
    [
        format!("{home}/.local/share/applications"),
        format!("{home}/.local/share/flatpak/exports/share/applications"),
    ]
}

/// The desktop file id of `relative`, a path below an applications folder:
/// `kde4/okular.desktop` is `kde4-okular.desktop`.
pub fn desktop_id(relative: &str) -> String {
    relative.replace('/', "-")
}

/// Whether a change inside `folder` can change the apps: it is an applications folder or is
/// inside one.
pub fn is_app_folder(folder: &str) -> bool {
    folder.ends_with("/applications") || folder.contains("/applications/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_folders_come_from_the_home() {
        assert_eq!(
            user_app_dirs("/home/ana/"),
            [
                "/home/ana/.local/share/applications",
                "/home/ana/.local/share/flatpak/exports/share/applications",
            ]
        );
    }

    #[test]
    fn ids_flatten_subfolders() {
        assert_eq!(desktop_id("firefox.desktop"), "firefox.desktop");
        assert_eq!(desktop_id("kde4/okular.desktop"), "kde4-okular.desktop");
    }

    #[test]
    fn app_folders_are_recognized() {
        assert!(is_app_folder("/usr/share/applications"));
        assert!(is_app_folder("/usr/share/applications/kde4"));
        assert!(is_app_folder("/home/ana/.local/share/applications"));
        assert!(!is_app_folder("/usr/share/icons"));
        assert!(!is_app_folder("/home/ana/applications-old"));
    }
}
