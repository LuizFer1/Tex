use tex_ipc::{Hit, VolumeState, VolumeStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Apps,
    Folders,
    Files,
}

impl Category {
    pub fn index(self) -> i32 {
        match self {
            Category::Apps => 0,
            Category::Folders => 1,
            Category::Files => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(Category),
    Item {
        name: String,
        folder: String,
        path: String,
        is_dir: bool,
        is_app: bool,
    },
}

impl Row {
    pub fn is_item(&self) -> bool {
        matches!(self, Row::Item { .. })
    }
}

/// Groups hits under apps, folders and files, keeping the service's order inside each group.
/// Apps always come first; of the other two, the group holding the better hit comes first.
pub fn build_rows(hits: &[Hit]) -> Vec<Row> {
    let (apps, others): (Vec<&Hit>, Vec<&Hit>) = hits.iter().partition(|hit| hit.is_app);
    let (folders, files): (Vec<&Hit>, Vec<&Hit>) =
        others.iter().copied().partition(|hit| hit.is_dir);
    let mut groups = [(Category::Folders, folders), (Category::Files, files)];
    if !others.first().is_some_and(|hit| hit.is_dir) {
        groups.swap(0, 1);
    }
    let mut rows = Vec::new();
    for (category, group) in std::iter::once((Category::Apps, apps)).chain(groups) {
        if group.is_empty() {
            continue;
        }
        rows.push(Row::Header(category));
        rows.extend(group.into_iter().map(item));
    }
    rows
}

fn item(hit: &Hit) -> Row {
    let (folder, name) = match hit.path.rsplit_once('\\') {
        Some((folder, name)) if folder.ends_with(':') => (format!("{folder}\\"), name),
        Some((folder, name)) => (folder.to_owned(), name),
        None => (String::new(), hit.path.as_str()),
    };
    let name = if hit.is_app {
        strip_app_extension(name)
    } else {
        name
    };
    Row::Item {
        name: name.to_owned(),
        folder,
        path: hit.path.clone(),
        is_dir: hit.is_dir,
        is_app: hit.is_app,
    }
}

fn strip_app_extension(name: &str) -> &str {
    [".lnk", ".url", ".exe"]
        .iter()
        .find_map(|extension| {
            let cut = name
                .len()
                .checked_sub(extension.len())
                .filter(|&cut| cut > 0)?;
            name.get(cut..)?
                .eq_ignore_ascii_case(extension)
                .then(|| &name[..cut])
        })
        .unwrap_or(name)
}

/// Moves the selection by `step` (+1 or -1) to the next item row, skipping headers. Without a
/// selection it starts from the end it moves away from; at either end it stays where it was.
pub fn step_selection(is_item: &[bool], current: Option<usize>, step: isize) -> Option<usize> {
    let mut index = match current {
        Some(current) => current as isize + step,
        None if step > 0 => 0,
        None => is_item.len() as isize - 1,
    };
    while index >= 0 && (index as usize) < is_item.len() {
        if is_item[index as usize] {
            return Some(index as usize);
        }
        index += step;
    }
    current
}

/// Volumes that are not ready, without words so the UI can translate the sentence:
/// ("C: 45%, D: 3%", "E: F:"). Both empty when every volume is ready.
pub fn status_parts(volumes: &[VolumeStatus]) -> (String, String) {
    let indexing: Vec<String> = volumes
        .iter()
        .filter_map(|volume| match volume.state {
            VolumeState::Indexing { percent } => Some(format!("{} {percent}%", volume.root)),
            _ => None,
        })
        .collect();
    let failed: Vec<String> = volumes
        .iter()
        .filter(|volume| matches!(volume.state, VolumeState::Failed { .. }))
        .map(|volume| volume.root.clone())
        .collect();
    (indexing.join(", "), failed.join(" "))
}

/// "1234567" with a thousands separator: "1.234.567".
pub fn group_digits(value: u64, separator: char) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(separator);
        }
        grouped.push(digit);
    }
    grouped
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIcon {
    File,
    Doc,
    Sheet,
    Pdf,
    Text,
    Image,
    Archive,
    Code,
}

impl FileIcon {
    pub fn index(self) -> i32 {
        self as i32
    }
}

/// Picks the icon kind from the file extension.
pub fn file_icon(name: &str) -> FileIcon {
    let extension = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extension.to_ascii_lowercase(),
        _ => return FileIcon::File,
    };
    match extension.as_str() {
        "doc" | "docx" | "odt" | "rtf" => FileIcon::Doc,
        "xls" | "xlsx" | "xlsm" | "csv" | "ods" => FileIcon::Sheet,
        "pdf" => FileIcon::Pdf,
        "txt" | "md" | "log" | "ini" | "cfg" => FileIcon::Text,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff"
        | "heic" => FileIcon::Image,
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" => FileIcon::Archive,
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cs" | "java"
        | "go" | "json" | "toml" | "yaml" | "yml" | "xml" | "html" | "css" | "sh" | "ps1"
        | "bat" | "slint" => FileIcon::Code,
        _ => FileIcon::File,
    }
}

/// The last three folders after "…\", or the path as is when it has three or fewer.
pub fn short_path(folder: &str) -> String {
    let parts: Vec<&str> = folder.split('\\').filter(|part| !part.is_empty()).collect();
    if parts.len() <= 4 {
        return folder.to_string();
    }
    format!("…\\{}", parts[parts.len() - 3..].join("\\"))
}

/// For each row, the number of items in its group when it is a header, 0 otherwise.
pub fn header_counts(rows: &[Row]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| match row {
            Row::Header(_) => rows[index + 1..]
                .iter()
                .take_while(|row| row.is_item())
                .count(),
            Row::Item { .. } => 0,
        })
        .collect()
}

/// Ready volumes as "C: D:" (or "/ /home") and their entries added up.
pub fn ready_summary(volumes: &[VolumeStatus]) -> (String, u64) {
    let ready: Vec<(&str, u32)> = volumes
        .iter()
        .filter_map(|volume| match volume.state {
            VolumeState::Ready { entries } => Some((volume.root.as_str(), entries)),
            _ => None,
        })
        .collect();
    let roots: Vec<&str> = ready.iter().map(|(root, _)| *root).collect();
    (
        roots.join(" "),
        ready.iter().map(|(_, entries)| u64::from(*entries)).sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_ipc::VolumeState;

    fn hit(path: &str, is_dir: bool) -> Hit {
        Hit {
            path: path.into(),
            is_dir,
            is_app: false,
            label: None,
        }
    }

    fn item(name: &str, folder: &str, path: &str, is_dir: bool) -> Row {
        Row::Item {
            name: name.into(),
            folder: folder.into(),
            path: path.into(),
            is_dir,
            is_app: false,
        }
    }

    #[test]
    fn apps_come_first_without_their_extension() {
        let app = |path: &str| Hit {
            path: path.into(),
            is_dir: false,
            is_app: true,
            label: None,
        };
        let rows = build_rows(&[
            hit(r"C:\Projetos\code", true),
            app(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk"),
            hit(r"C:\notes.code", false),
            app(r"C:\Program Files\Code\Code.exe"),
        ]);
        assert_eq!(
            rows,
            [
                Row::Header(Category::Apps),
                Row::Item {
                    name: "Visual Studio Code".into(),
                    folder: r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs".into(),
                    path: r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk".into(),
                    is_dir: false,
                    is_app: true,
                },
                Row::Item {
                    name: "Code".into(),
                    folder: r"C:\Program Files\Code".into(),
                    path: r"C:\Program Files\Code\Code.exe".into(),
                    is_dir: false,
                    is_app: true,
                },
                Row::Header(Category::Folders),
                item("code", r"C:\Projetos", r"C:\Projetos\code", true),
                Row::Header(Category::Files),
                item("notes.code", r"C:\", r"C:\notes.code", false),
            ]
        );
    }

    #[test]
    fn files_come_first_when_the_best_hit_is_a_file() {
        let rows = build_rows(&[
            hit(r"C:\Docs\notes.txt", false),
            hit(r"C:\notes", true),
            hit(r"D:\notes.md", false),
        ]);
        assert_eq!(
            rows,
            [
                Row::Header(Category::Files),
                item("notes.txt", r"C:\Docs", r"C:\Docs\notes.txt", false),
                item("notes.md", r"D:\", r"D:\notes.md", false),
                Row::Header(Category::Folders),
                item("notes", r"C:\", r"C:\notes", true),
            ]
        );
    }

    #[test]
    fn folders_come_first_when_the_best_hit_is_a_folder() {
        let rows = build_rows(&[hit(r"C:\notes", true), hit(r"C:\notes.txt", false)]);
        assert_eq!(rows[0], Row::Header(Category::Folders));
        assert_eq!(rows[2], Row::Header(Category::Files));
    }

    #[test]
    fn no_hits_no_rows() {
        assert!(build_rows(&[]).is_empty());
    }

    #[test]
    fn selection_skips_headers_and_stops_at_the_ends() {
        let is_item = [false, true, true, false, true];
        assert_eq!(step_selection(&is_item, None, 1), Some(1));
        assert_eq!(step_selection(&is_item, None, -1), Some(4));
        assert_eq!(step_selection(&is_item, Some(2), 1), Some(4));
        assert_eq!(step_selection(&is_item, Some(4), -1), Some(2));
        assert_eq!(step_selection(&is_item, Some(4), 1), Some(4));
        assert_eq!(step_selection(&is_item, Some(1), -1), Some(1));
        assert_eq!(step_selection(&[], None, 1), None);
    }

    #[test]
    fn status_parts_list_indexing_and_failed_volumes() {
        let volume = |root: &str, state| VolumeStatus {
            root: root.into(),
            state,
        };
        assert_eq!(
            status_parts(&[volume("C:", VolumeState::Ready { entries: 1 })]),
            (String::new(), String::new())
        );
        assert_eq!(
            status_parts(&[
                volume("C:", VolumeState::Indexing { percent: 45 }),
                volume("D:", VolumeState::Indexing { percent: 3 }),
                volume(
                    "E:",
                    VolumeState::Failed {
                        reason: "gone".into()
                    }
                ),
                volume(
                    "F:",
                    VolumeState::Failed {
                        reason: "gone".into()
                    }
                ),
            ]),
            ("C: 45%, D: 3%".into(), "E: F:".into())
        );
        assert_eq!(
            status_parts(&[
                volume("/home", VolumeState::Indexing { percent: 12 }),
                volume(
                    "/",
                    VolumeState::Failed {
                        reason: "gone".into()
                    }
                ),
            ]),
            ("/home 12%".into(), "/".into())
        );
    }

    #[test]
    fn group_digits_inserts_separators() {
        assert_eq!(group_digits(0, '.'), "0");
        assert_eq!(group_digits(999, '.'), "999");
        assert_eq!(group_digits(1000, '.'), "1.000");
        assert_eq!(group_digits(1_234_567, ','), "1,234,567");
    }

    #[test]
    fn short_path_keeps_the_last_three_folders() {
        assert_eq!(
            short_path(r"C:\Users\ana\Documents\Work\2026"),
            r"…\Documents\Work\2026"
        );
        assert_eq!(
            short_path(r"C:\Program Files\Steam"),
            r"C:\Program Files\Steam"
        );
        assert_eq!(short_path(r"D:\"), r"D:\");
        assert_eq!(short_path(r"C:\a\b\c\"), r"C:\a\b\c\");
    }

    #[test]
    fn file_icon_by_extension() {
        assert_eq!(file_icon("Relatório.DOCX"), FileIcon::Doc);
        assert_eq!(file_icon("dados.csv"), FileIcon::Sheet);
        assert_eq!(file_icon("nota.pdf"), FileIcon::Pdf);
        assert_eq!(file_icon("leia.md"), FileIcon::Text);
        assert_eq!(file_icon("foto.JPEG"), FileIcon::Image);
        assert_eq!(file_icon("backup.7z"), FileIcon::Archive);
        assert_eq!(file_icon("main.rs"), FileIcon::Code);
        assert_eq!(file_icon("Makefile"), FileIcon::File);
        assert_eq!(file_icon(".gitignore"), FileIcon::File);
    }

    #[test]
    fn header_counts_count_items_of_each_group() {
        let item = || Row::Item {
            name: String::new(),
            folder: String::new(),
            path: String::new(),
            is_dir: false,
            is_app: false,
        };
        let rows = vec![
            Row::Header(Category::Apps),
            item(),
            item(),
            Row::Header(Category::Files),
            item(),
        ];
        assert_eq!(header_counts(&rows), [2, 0, 0, 1, 0]);
    }

    #[test]
    fn ready_summary_lists_ready_letters_and_adds_entries() {
        let volumes = [
            VolumeStatus {
                root: "C:".into(),
                state: VolumeState::Ready {
                    entries: 4_000_000_000,
                },
            },
            VolumeStatus {
                root: "D:".into(),
                state: VolumeState::Indexing { percent: 3 },
            },
            VolumeStatus {
                root: "E:".into(),
                state: VolumeState::Ready {
                    entries: 500_000_000,
                },
            },
        ];
        assert_eq!(
            ready_summary(&volumes),
            ("C: E:".to_string(), 4_500_000_000)
        );
        assert_eq!(ready_summary(&[]), (String::new(), 0));
    }

    #[test]
    fn group_digits_handles_totals_above_u32() {
        assert_eq!(group_digits(4_500_000_000, '.'), "4.500.000.000");
    }
}
