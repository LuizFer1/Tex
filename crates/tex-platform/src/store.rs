pub const STORE_PREFIX: &str = r"shell:AppsFolder\";

/// An app of the Microsoft Store: the name the shell shows and the parsing name that opens it
/// (`shell:AppsFolder\<AppUserModelID>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreApp {
    pub name: String,
    pub path: String,
}

/// The app for one `AppsFolder` item, or `None` when it is not a packaged app (its
/// AppUserModelID has no `!`) or has no name.
pub fn store_app(name: &str, parsing_name: &str) -> Option<StoreApp> {
    let id = parsing_name
        .strip_prefix(STORE_PREFIX)
        .unwrap_or(parsing_name);
    let name = name.trim();
    (id.contains('!') && !name.is_empty()).then(|| StoreApp {
        name: name.to_owned(),
        path: format!("{STORE_PREFIX}{id}"),
    })
}

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::core::PWSTR;

/// Every packaged app of the current user, sorted by name. Initializes COM on the calling
/// thread. Empty when the shell cannot be read.
pub fn store_apps() -> Vec<StoreApp> {
    let mut apps = read_apps().unwrap_or_default();
    apps.sort_by_cached_key(|app| app.name.to_lowercase());
    apps
}

fn read_apps() -> windows::core::Result<Vec<StoreApp>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut apps = Vec::new();
        loop {
            let mut batch = [None];
            let mut fetched = 0;
            items.Next(&mut batch, Some(&mut fetched))?;
            let Some(item) = batch[0].take().filter(|_| fetched == 1) else {
                break;
            };
            let (Some(name), Some(parsing)) = (
                display_name(&item, SIGDN_NORMALDISPLAY),
                display_name(&item, SIGDN_DESKTOPABSOLUTEPARSING),
            ) else {
                continue;
            };
            apps.extend(store_app(&name, &parsing));
        }
        Ok(apps)
    }
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let text: PWSTR = item.GetDisplayName(kind).ok()?;
        let value = text.to_string().ok();
        CoTaskMemFree(Some(text.0.cast()));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_apps_keep_their_id() {
        let app = store_app(
            "Windows Terminal",
            r"shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
        )
        .unwrap();
        assert_eq!(app.name, "Windows Terminal");
        assert_eq!(
            app.path,
            r"shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"
        );
    }

    #[test]
    fn id_without_the_prefix_gets_it() {
        let app = store_app(
            "Calculadora",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
        )
        .unwrap();
        assert_eq!(
            app.path,
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        );
    }

    #[test]
    fn win32_items_and_nameless_items_are_not_store_apps() {
        assert!(
            store_app(
                "Notepad++",
                r"shell:AppsFolder\{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\Notepad++\notepad++.exe"
            )
            .is_none()
        );
        assert!(store_app("  ", r"shell:AppsFolder\X_8wekyb3d8bbwe!App").is_none());
    }

    #[test]
    fn the_shell_lists_some_packaged_apps() {
        let apps = store_apps();
        assert!(
            apps.iter()
                .all(|app| app.path.starts_with(STORE_PREFIX) && app.path.contains('!'))
        );
        assert!(
            apps.windows(2)
                .all(|pair| pair[0].name.to_lowercase() <= pair[1].name.to_lowercase())
        );
    }
}
