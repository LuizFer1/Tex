use tex_ipc::{Hit, VolumeState, VolumeStatus};

pub const SERVICE_UNAVAILABLE: &str =
    "Search service is not running. Run tex-indexer install in an administrator terminal.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    Item {
        name: String,
        folder: String,
        path: String,
        is_dir: bool,
    },
}

impl Row {
    pub fn is_item(&self) -> bool {
        matches!(self, Row::Item { .. })
    }
}

/// Groups hits under "Folders" and "Files", keeping the service's order inside each group.
/// The group that holds the best (first) hit comes first.
pub fn build_rows(hits: &[Hit]) -> Vec<Row> {
    let (folders, files): (Vec<&Hit>, Vec<&Hit>) = hits.iter().partition(|hit| hit.is_dir);
    let mut groups = [("Folders", folders), ("Files", files)];
    if !hits.first().is_some_and(|hit| hit.is_dir) {
        groups.swap(0, 1);
    }
    let mut rows = Vec::new();
    for (title, group) in groups {
        if group.is_empty() {
            continue;
        }
        rows.push(Row::Header(title));
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
    Row::Item {
        name: name.to_owned(),
        folder,
        path: hit.path.clone(),
        is_dir: hit.is_dir,
    }
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

/// One line about volumes that are not ready, e.g. "Indexing C: 45%… · Unavailable: E:". Empty when all are ready.
pub fn status_text(volumes: &[VolumeStatus]) -> String {
    let indexing: Vec<String> = volumes
        .iter()
        .filter_map(|volume| match volume.state {
            VolumeState::Indexing { percent } => Some(format!("{}: {percent}%", volume.letter)),
            _ => None,
        })
        .collect();
    let failed: Vec<String> = volumes
        .iter()
        .filter(|volume| matches!(volume.state, VolumeState::Failed { .. }))
        .map(|volume| format!("{}:", volume.letter))
        .collect();
    let mut parts = Vec::new();
    if !indexing.is_empty() {
        parts.push(format!("Indexing {}…", indexing.join(", ")));
    }
    if !failed.is_empty() {
        parts.push(format!("Unavailable: {}", failed.join(" ")));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_ipc::VolumeState;

    fn hit(path: &str, is_dir: bool) -> Hit {
        Hit {
            path: path.into(),
            is_dir,
        }
    }

    fn item(name: &str, folder: &str, path: &str, is_dir: bool) -> Row {
        Row::Item {
            name: name.into(),
            folder: folder.into(),
            path: path.into(),
            is_dir,
        }
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
                Row::Header("Files"),
                item("notes.txt", r"C:\Docs", r"C:\Docs\notes.txt", false),
                item("notes.md", r"D:\", r"D:\notes.md", false),
                Row::Header("Folders"),
                item("notes", r"C:\", r"C:\notes", true),
            ]
        );
    }

    #[test]
    fn folders_come_first_when_the_best_hit_is_a_folder() {
        let rows = build_rows(&[hit(r"C:\notes", true), hit(r"C:\notes.txt", false)]);
        assert_eq!(rows[0], Row::Header("Folders"));
        assert_eq!(rows[2], Row::Header("Files"));
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
    fn status_text_lists_volumes_that_are_not_ready() {
        let volume = |letter, state| VolumeStatus { letter, state };
        assert_eq!(
            status_text(&[volume('C', VolumeState::Ready { entries: 1 })]),
            ""
        );
        assert_eq!(
            status_text(&[
                volume('C', VolumeState::Indexing { percent: 45 }),
                volume('D', VolumeState::Indexing { percent: 3 }),
            ]),
            "Indexing C: 45%, D: 3%…"
        );
        assert_eq!(
            status_text(&[
                volume('C', VolumeState::Indexing { percent: 45 }),
                volume(
                    'E',
                    VolumeState::Failed {
                        reason: "gone".into()
                    }
                ),
            ]),
            "Indexing C: 45%… · Unavailable: E:"
        );
    }
}
