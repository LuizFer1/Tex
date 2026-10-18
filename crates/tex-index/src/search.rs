use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use crate::apps::AppCatalog;
use crate::entry::{ROOT, Record};
use crate::matcher::{MatchKind, Query};
use crate::normalize::fold_into;
use crate::volume::{FolderMask, VolumeIndex};

const CHUNK: usize = 64 * 1024;
const START_MENU_APP: u8 = 0;
const EXECUTABLE_APP: u8 = 1;
const NOT_APP: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Apps,
    Files,
    Folders,
}

pub struct SearchRequest<'a> {
    pub query: &'a Query,
    pub filter: Filter,
    pub limit: usize,
    pub home: Option<&'a str>,
    pub include_hidden: bool,
    pub include_system: bool,
    pub excluded_folders: &'a [&'a str],
    pub categories: Categories,
    pub app_roots: &'a [&'a str],
    /// Apps from `.desktop` files (Linux); `None` where apps come from the index (Windows).
    pub apps: Option<&'a AppCatalog>,
    /// The user's language ("pt-BR"), for the apps' localized names.
    pub language: &'a str,
}

/// Which kinds of results a search may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Categories {
    pub apps: bool,
    pub files: bool,
    pub folders: bool,
}

impl Categories {
    pub const ALL: Self = Self {
        apps: true,
        files: true,
        folders: true,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub is_dir: bool,
    pub is_app: bool,
    /// The app's display name, for apps from the catalog.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub total: u32,
}

type Key = (u8, Reverse<MatchKind>, Reverse<MatchKind>, bool, u16, u16);

struct Prepared<'a> {
    volume: &'a VolumeIndex,
    home: Option<Record>,
    app_roots: Vec<Record>,
    excluded: Vec<Record>,
    folders: Vec<u8>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    key: Key,
    name: String,
    volume: usize,
    record: Record,
    is_dir: bool,
    is_app: bool,
}

/// Searches all volumes in parallel and returns the best `limit` hits, best first.
/// Returns `None` if `cancelled` became true before the search finished.
pub fn search(
    volumes: &[&VolumeIndex],
    request: &SearchRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<SearchOutcome> {
    let query = request.query;
    let app_hits = match request.apps {
        Some(apps) if offers_catalog_apps(request) => {
            apps.search(query, request.language, request.home)
        }
        _ => Vec::new(),
    };
    let volumes: Vec<&VolumeIndex> = volumes
        .iter()
        .copied()
        .filter(|volume| {
            query
                .drive()
                .is_none_or(|drive| volume.drive() == Some(drive))
        })
        .collect();
    let parallelism = thread::available_parallelism().map_or(1, |n| n.get());
    let masks = folder_masks(&volumes, query, parallelism, cancelled)?;
    let prepared: Vec<Prepared> = volumes
        .into_iter()
        .zip(masks)
        .filter(|(_, folders)| covers(folders, query.folder_count()))
        .map(|(volume, folders)| Prepared {
            volume,
            home: request.home.and_then(|home| volume.cached_lookup(home)),
            app_roots: request
                .app_roots
                .iter()
                .filter_map(|root| volume.cached_lookup(root))
                .collect(),
            excluded: request
                .excluded_folders
                .iter()
                .filter_map(|folder| volume.cached_lookup(folder))
                .collect(),
            folders,
        })
        .collect();
    let chunks: Vec<(usize, usize)> = prepared
        .iter()
        .enumerate()
        .flat_map(|(v, target)| {
            (0..target.volume.entries().len())
                .step_by(CHUNK)
                .map(move |start| (v, start))
        })
        .collect();
    let next = AtomicUsize::new(0);
    let threads = parallelism.min(chunks.len().max(1));

    let results: Vec<Option<Worker>> = thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut worker = Worker::new(request);
                    loop {
                        if cancelled() {
                            return None;
                        }
                        let Some(&(v, start)) = chunks.get(next.fetch_add(1, Ordering::Relaxed))
                        else {
                            return Some(worker);
                        };
                        worker.scan(&prepared[v], v, start);
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("search worker panicked"))
            .collect()
    });

    let mut total = app_hits.len() as u32;
    let mut candidates = Vec::new();
    for worker in results {
        let worker = worker?;
        total += worker.total;
        candidates.extend(worker.heap.into_vec());
    }
    candidates.sort();
    candidates.truncate(request.limit.saturating_sub(app_hits.len()));
    let mut hits = app_hits;
    hits.truncate(request.limit);
    hits.extend(candidates.into_iter().filter_map(|c| {
        Some(Hit {
            path: prepared[c.volume].volume.path(c.record)?,
            is_dir: c.is_dir,
            is_app: c.is_app,
            label: None,
        })
    }));
    Some(SearchOutcome { hits, total })
}

/// Catalog apps answer queries for a name alone, when apps may be returned.
fn offers_catalog_apps(request: &SearchRequest) -> bool {
    request.categories.apps
        && matches!(request.filter, Filter::All | Filter::Apps)
        && request.query.folder_count() == 0
        && request.query.drive().is_none()
}

/// For each volume, one byte per entry with bit `i` set on directories matching folder `i` of
/// the query, filled in parallel. Empty vectors when the query has no folders. `None` if
/// cancelled.
fn folder_masks(
    volumes: &[&VolumeIndex],
    query: &Query,
    parallelism: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<Vec<u8>>> {
    let mut masks: Vec<Vec<u8>> = vec![Vec::new(); volumes.len()];
    if query.folder_count() == 0 {
        return Some(masks);
    }
    for (mask, volume) in masks.iter_mut().zip(volumes) {
        mask.resize(volume.entries().len(), 0);
    }
    let jobs: Vec<(usize, usize, &mut [u8])> = masks
        .iter_mut()
        .enumerate()
        .flat_map(|(v, mask)| {
            mask.chunks_mut(CHUNK)
                .enumerate()
                .map(move |(i, bits)| (v, i * CHUNK, bits))
        })
        .collect();
    let threads = parallelism.min(jobs.len().max(1));
    let jobs = Mutex::new(jobs);
    let stopped = AtomicBool::new(false);
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut scratch = String::new();
                loop {
                    if cancelled() {
                        stopped.store(true, Ordering::Relaxed);
                        return;
                    }
                    let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).pop();
                    let Some((v, start, bits)) = job else {
                        return;
                    };
                    mark_folders(volumes[v], query, start, bits, &mut scratch);
                }
            });
        }
    });
    drop(jobs);
    (!stopped.into_inner()).then_some(masks)
}

fn mark_folders(
    volume: &VolumeIndex,
    query: &Query,
    start: usize,
    mask: &mut [u8],
    scratch: &mut String,
) {
    let entries = &volume.entries()[start..start + mask.len()];
    for (bits, entry) in mask.iter_mut().zip(entries) {
        if !entry.in_use() || !entry.is_dir() {
            continue;
        }
        let name = volume.entry_name(entry);
        for folder in 0..query.folder_count() {
            if query.folder_kind(folder, name, scratch).is_some() {
                *bits |= 1 << folder;
            }
        }
    }
}

fn covers(mask: &[u8], count: usize) -> bool {
    count == 0 || mask.iter().fold(0, |all, bits| all | bits) == u8::MAX >> (8 - count)
}

struct Worker<'a> {
    request: &'a SearchRequest<'a>,
    heap: BinaryHeap<Candidate>,
    total: u32,
    scratch: String,
}

impl<'a> Worker<'a> {
    fn new(request: &'a SearchRequest<'a>) -> Self {
        Self {
            request,
            heap: BinaryHeap::new(),
            total: 0,
            scratch: String::new(),
        }
    }

    fn scan(&mut self, target: &Prepared, volume_index: usize, start: usize) {
        let volume = target.volume;
        let query = self.request.query;
        let count = query.folder_count();
        let folders = (count > 0).then(|| FolderMask {
            bits: &target.folders,
            count: count as u8,
        });
        let entries = volume.entries();
        let end = (start + CHUNK).min(entries.len());
        for (offset, entry) in entries[start..end].iter().enumerate() {
            let record = (start + offset) as Record;
            if !entry.in_use() || record == ROOT {
                continue;
            }
            let wants_dir = self.request.filter == Filter::Folders;
            if self.request.filter != Filter::All && entry.is_dir() != wants_dir {
                continue;
            }
            let name = volume.entry_name(entry);
            let Some(kind) = query.matches(name, &mut self.scratch) else {
                continue;
            };
            let Some(lineage) = volume.lineage(
                record,
                target.home,
                &target.app_roots,
                &target.excluded,
                folders,
            ) else {
                continue;
            };
            if (lineage.hidden && !self.request.include_hidden)
                || (lineage.system && !self.request.include_system)
                || lineage.excluded
                || !lineage.in_folders
            {
                continue;
            }
            let app = if volume.is_unix() {
                NOT_APP
            } else {
                app_rank(entry.is_dir(), name, lineage.in_app_root)
            };
            let is_app = app != NOT_APP;
            let categories = self.request.categories;
            let shown = if is_app {
                categories.apps
            } else if entry.is_dir() {
                categories.folders
            } else {
                categories.files
            };
            if !shown {
                continue;
            }
            if (self.request.filter == Filter::Apps && !is_app)
                || (self.request.filter == Filter::Files && is_app)
            {
                continue;
            }
            self.total += 1;
            let folder = lineage
                .nearest
                .and_then(|nearest| volume.entry(nearest))
                .and_then(|nearest| {
                    query.folder_kind(count - 1, volume.entry_name(nearest), &mut self.scratch)
                })
                .unwrap_or(MatchKind::Exact);
            let key = (
                app,
                Reverse(kind),
                Reverse(folder),
                !lineage.in_home,
                name.len() as u16,
                lineage.depth,
            );
            self.offer(key, name, volume_index, record, entry.is_dir(), is_app);
        }
    }

    fn offer(
        &mut self,
        key: Key,
        name: &[u8],
        volume: usize,
        record: Record,
        is_dir: bool,
        is_app: bool,
    ) {
        let limit = self.request.limit;
        if limit == 0 {
            return;
        }
        if self.heap.len() >= limit && self.heap.peek().is_some_and(|worst| key > worst.key) {
            return;
        }
        let mut folded = String::new();
        fold_into(std::str::from_utf8(name).unwrap_or_default(), &mut folded);
        self.heap.push(Candidate {
            key,
            name: folded,
            volume,
            record,
            is_dir,
            is_app,
        });
        if self.heap.len() > limit {
            self.heap.pop();
        }
    }
}

/// 0 for a shortcut (.lnk or .url) inside a Start Menu folder, 1 for an .exe, 2 for anything else.
fn app_rank(is_dir: bool, name: &[u8], in_app_root: bool) -> u8 {
    if is_dir {
        NOT_APP
    } else if in_app_root && (has_extension(name, b".lnk") || has_extension(name, b".url")) {
        START_MENU_APP
    } else if has_extension(name, b".exe") {
        EXECUTABLE_APP
    } else {
        NOT_APP
    }
}

fn has_extension(name: &[u8], extension: &[u8]) -> bool {
    name.len() > extension.len()
        && name[name.len() - extension.len()..].eq_ignore_ascii_case(extension)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{AppCatalog, DesktopApp};
    use crate::entry::{Attributes, ROOT};

    const FILE: Attributes = Attributes {
        dir: false,
        hidden: false,
        system: false,
    };
    const DIR: Attributes = Attributes {
        dir: true,
        hidden: false,
        system: false,
    };

    fn drive_c() -> VolumeIndex {
        let mut c = VolumeIndex::new("C:");
        c.upsert(20, ROOT, "Users", DIR);
        c.upsert(21, 20, "luis", DIR);
        c.upsert(22, 21, "Documents", DIR);
        c.upsert(23, 22, "Relatório final.pdf", FILE);
        c.upsert(
            24,
            21,
            "AppData",
            Attributes {
                hidden: true,
                ..DIR
            },
        );
        c.upsert(25, 24, "relatorio.tmp", FILE);
        c.upsert(26, 22, "notes.txt", FILE);
        c.upsert(30, ROOT, "Windows", DIR);
        c.upsert(31, 30, "relatorio.log", FILE);
        c.set_windows_dir(Some(30));
        c.upsert(
            32,
            ROOT,
            "$Recycle.Bin",
            Attributes {
                dir: true,
                hidden: true,
                system: true,
            },
        );
        c.upsert(33, 32, "relatorio.bak", FILE);
        c.upsert(40, ROOT, "Projetos", DIR);
        c.upsert(41, 40, "relatorio", DIR);
        c.upsert(42, 40, "old_relatorio.pdf", FILE);
        c.upsert(43, 40, "notes.txt", FILE);
        c
    }

    fn run(
        volumes: &[&VolumeIndex],
        text: &str,
        filter: Filter,
        limit: usize,
        home: Option<&str>,
        include_hidden: bool,
    ) -> SearchOutcome {
        let query = Query::parse(text).unwrap();
        let request = SearchRequest {
            filter,
            limit,
            home,
            include_hidden,
            ..base(&query)
        };
        search(volumes, &request, &|| false).unwrap()
    }

    fn base(query: &Query) -> SearchRequest<'_> {
        SearchRequest {
            query,
            filter: Filter::All,
            limit: 10,
            home: None,
            include_hidden: false,
            include_system: false,
            excluded_folders: &[],
            categories: Categories::ALL,
            app_roots: &[],
            apps: None,
            language: "",
        }
    }

    fn found(volume: &VolumeIndex, request: &SearchRequest) -> Vec<String> {
        search(&[volume], request, &|| false)
            .unwrap()
            .hits
            .into_iter()
            .map(|hit| hit.path)
            .collect()
    }

    fn paths(outcome: &SearchOutcome) -> Vec<&str> {
        outcome.hits.iter().map(|hit| hit.path.as_str()).collect()
    }

    #[test]
    fn ranks_exact_then_prefix_then_word_start() {
        let c = drive_c();
        let outcome = run(
            &[&c],
            "relatorio",
            Filter::All,
            10,
            Some(r"C:\Users\luis"),
            false,
        );
        assert_eq!(
            paths(&outcome),
            [
                r"C:\Projetos\relatorio",
                r"C:\Users\luis\Documents\Relatório final.pdf",
                r"C:\Projetos\old_relatorio.pdf",
            ]
        );
        assert_eq!(outcome.total, 3);
        assert!(outcome.hits[0].is_dir);
        assert!(!outcome.hits[1].is_dir);
    }

    #[test]
    fn hidden_and_system_are_separate() {
        let c = drive_c();
        let query = Query::parse("relatorio").unwrap();
        let with = |include_hidden, include_system| {
            found(
                &c,
                &SearchRequest {
                    include_hidden,
                    include_system,
                    ..base(&query)
                },
            )
        };
        let tmp = r"C:\Users\luis\AppData\relatorio.tmp".to_string();
        let log = r"C:\Windows\relatorio.log".to_string();
        let bak = r"C:\$Recycle.Bin\relatorio.bak".to_string();
        assert_eq!(with(false, false).len(), 3);
        let hidden = with(true, false);
        assert!(hidden.contains(&tmp) && !hidden.contains(&log) && !hidden.contains(&bak));
        let system = with(false, true);
        assert!(!system.contains(&tmp) && system.contains(&log) && !system.contains(&bak));
        let both = with(true, true);
        assert!(both.contains(&tmp) && both.contains(&log) && both.contains(&bak));
    }

    #[test]
    fn unix_hidden_and_system_follow_names() {
        let mut root = VolumeIndex::new("/");
        root.upsert(20, ROOT, "home", DIR);
        root.upsert(21, 20, "luis", DIR);
        root.upsert(22, 21, ".notes.txt", FILE);
        root.upsert(23, 21, "notes.txt", FILE);
        root.upsert(24, ROOT, "etc", DIR);
        root.upsert(25, 24, "notes.conf", FILE);
        let query = Query::parse_for("notes", false).unwrap();
        assert_eq!(found(&root, &base(&query)), ["/home/luis/notes.txt"]);
        let everything = SearchRequest {
            include_hidden: true,
            include_system: true,
            ..base(&query)
        };
        assert_eq!(found(&root, &everything).len(), 3);
    }

    #[test]
    fn excluded_folder_hides_descendants() {
        let c = drive_c();
        let query = Query::parse("relatorio").unwrap();
        let request = SearchRequest {
            excluded_folders: &[r"C:\Projetos"],
            ..base(&query)
        };
        assert_eq!(
            found(&c, &request),
            [r"C:\Users\luis\Documents\Relatório final.pdf"]
        );
    }

    #[test]
    fn excluded_folder_itself_is_hidden() {
        let c = drive_c();
        let query = Query::parse("projetos").unwrap();
        let request = SearchRequest {
            excluded_folders: &[r"c:\projetos\"],
            ..base(&query)
        };
        assert!(found(&c, &request).is_empty());
    }

    #[test]
    fn disabled_categories_do_not_use_the_limit() {
        let c = drive_c();
        let query = Query::parse("relatorio").unwrap();
        let request = SearchRequest {
            limit: 1,
            categories: Categories {
                folders: false,
                ..Categories::ALL
            },
            ..base(&query)
        };
        let outcome = search(&[&c], &request, &|| false).unwrap();
        assert_eq!(
            paths(&outcome),
            [r"C:\Users\luis\Documents\Relatório final.pdf"]
        );
        assert_eq!(outcome.total, 2);
    }

    #[test]
    fn home_breaks_ties_before_depth() {
        let c = drive_c();
        let with_home = run(
            &[&c],
            "notes",
            Filter::All,
            10,
            Some(r"C:\Users\luis"),
            false,
        );
        assert_eq!(
            paths(&with_home),
            [
                r"C:\Users\luis\Documents\notes.txt",
                r"C:\Projetos\notes.txt"
            ]
        );
        let without_home = run(&[&c], "notes", Filter::All, 10, None, false);
        assert_eq!(
            paths(&without_home),
            [
                r"C:\Projetos\notes.txt",
                r"C:\Users\luis\Documents\notes.txt"
            ]
        );
    }

    #[test]
    fn filters_files_and_folders() {
        let c = drive_c();
        assert_eq!(
            paths(&run(&[&c], "relatorio", Filter::Folders, 10, None, false)),
            [r"C:\Projetos\relatorio"]
        );
        assert_eq!(
            paths(&run(&[&c], "relatorio", Filter::Files, 10, None, false)),
            [
                r"C:\Users\luis\Documents\Relatório final.pdf",
                r"C:\Projetos\old_relatorio.pdf"
            ]
        );
    }

    #[test]
    fn limit_cuts_hits_but_not_total() {
        let c = drive_c();
        let outcome = run(&[&c], "relatorio", Filter::All, 1, None, false);
        assert_eq!(paths(&outcome), [r"C:\Projetos\relatorio"]);
        assert_eq!(outcome.total, 3);
        assert!(
            run(&[&c], "relatorio", Filter::All, 0, None, false)
                .hits
                .is_empty()
        );
    }

    #[test]
    fn searches_every_volume() {
        let c = drive_c();
        let mut d = VolumeIndex::new("D:");
        d.upsert(20, ROOT, "relatorio.txt", FILE);
        let outcome = run(&[&c, &d], "relatorio", Filter::All, 10, None, false);
        assert_eq!(
            paths(&outcome),
            [
                r"C:\Projetos\relatorio",
                r"D:\relatorio.txt",
                r"C:\Users\luis\Documents\Relatório final.pdf",
                r"C:\Projetos\old_relatorio.pdf",
            ]
        );
    }

    #[test]
    fn cancelled_search_returns_none() {
        let c = drive_c();
        let query = Query::parse("relatorio").unwrap();
        assert!(search(&[&c], &base(&query), &|| true).is_none());
    }

    fn with_apps() -> VolumeIndex {
        let mut c = drive_c();
        c.upsert(
            50,
            ROOT,
            "ProgramData",
            Attributes {
                hidden: true,
                ..DIR
            },
        );
        c.upsert(51, 50, "Programs", DIR);
        c.upsert(52, 51, "Relatorio Studio.lnk", FILE);
        c.upsert(53, 40, "relatorio.exe", FILE);
        c.upsert(54, 24, "relatorio-helper.exe", FILE);
        c
    }

    fn run_apps(volume: &VolumeIndex, filter: Filter, app_roots: &[&str]) -> SearchOutcome {
        let query = Query::parse("relatorio").unwrap();
        let request = SearchRequest {
            filter,
            app_roots,
            ..base(&query)
        };
        search(&[volume], &request, &|| false).unwrap()
    }

    #[test]
    fn apps_come_first_even_with_weaker_matches() {
        let c = with_apps();
        let outcome = run_apps(&c, Filter::All, &[r"C:\ProgramData\Programs"]);
        assert_eq!(
            paths(&outcome),
            [
                r"C:\ProgramData\Programs\Relatorio Studio.lnk",
                r"C:\Projetos\relatorio.exe",
                r"C:\Projetos\relatorio",
                r"C:\Users\luis\Documents\Relatório final.pdf",
                r"C:\Projetos\old_relatorio.pdf",
            ]
        );
        let apps: Vec<bool> = outcome.hits.iter().map(|hit| hit.is_app).collect();
        assert_eq!(apps, [true, true, false, false, false]);
        assert_eq!(outcome.total, 5);
    }

    #[test]
    fn filters_apps_files_and_folders() {
        let c = with_apps();
        let roots = [r"C:\ProgramData\Programs"];
        assert_eq!(
            paths(&run_apps(&c, Filter::Apps, &roots)),
            [
                r"C:\ProgramData\Programs\Relatorio Studio.lnk",
                r"C:\Projetos\relatorio.exe"
            ]
        );
        assert_eq!(
            paths(&run_apps(&c, Filter::Files, &roots)),
            [
                r"C:\Users\luis\Documents\Relatório final.pdf",
                r"C:\Projetos\old_relatorio.pdf"
            ]
        );
        assert_eq!(
            paths(&run_apps(&c, Filter::Folders, &roots)),
            [r"C:\Projetos\relatorio"]
        );
    }

    #[test]
    fn shortcuts_outside_app_roots_stay_hidden() {
        let c = with_apps();
        let outcome = run_apps(&c, Filter::All, &[]);
        assert_eq!(outcome.total, 4);
        assert!(!paths(&outcome).contains(&r"C:\ProgramData\Programs\Relatorio Studio.lnk"));
    }

    #[test]
    fn folder_finds_items_at_any_depth() {
        let c = drive_c();
        let query = Query::parse("users/.pdf").unwrap();
        assert_eq!(
            found(&c, &base(&query)),
            [r"C:\Users\luis\Documents\Relatório final.pdf"]
        );
    }

    #[test]
    fn folders_must_appear_in_order() {
        let c = drive_c();
        let query = Query::parse(r"users\documents\notes").unwrap();
        assert_eq!(
            found(&c, &base(&query)),
            [r"C:\Users\luis\Documents\notes.txt"]
        );
        let reversed = Query::parse("documents/users/notes").unwrap();
        assert!(found(&c, &base(&reversed)).is_empty());
    }

    #[test]
    fn empty_name_lists_the_folder_contents() {
        let c = drive_c();
        let query = Query::parse("projetos/").unwrap();
        let mut paths = found(&c, &base(&query));
        paths.sort();
        assert_eq!(
            paths,
            [
                r"C:\Projetos\notes.txt",
                r"C:\Projetos\old_relatorio.pdf",
                r"C:\Projetos\relatorio",
            ]
        );
    }

    #[test]
    fn exact_folder_name_ranks_first() {
        let mut c = VolumeIndex::new("C:");
        c.upsert(20, ROOT, "TexLive", DIR);
        c.upsert(21, 20, "a.exe", FILE);
        c.upsert(22, ROOT, "Tex", DIR);
        c.upsert(23, 22, "b.exe", FILE);
        c.upsert(24, 22, "b.exe.config", FILE);
        let query = Query::parse("tex/.exe").unwrap();
        assert_eq!(
            found(&c, &base(&query)),
            [r"C:\Tex\b.exe", r"C:\TexLive\a.exe"]
        );
    }

    #[test]
    fn folders_keep_hidden_and_excluded_rules() {
        let c = drive_c();
        let query = Query::parse("luis/.tmp").unwrap();
        assert!(found(&c, &base(&query)).is_empty());
        let hidden = SearchRequest {
            include_hidden: true,
            ..base(&query)
        };
        assert_eq!(found(&c, &hidden), [r"C:\Users\luis\AppData\relatorio.tmp"]);
        let projetos = Query::parse("projetos/").unwrap();
        let excluded = SearchRequest {
            excluded_folders: &[r"C:\Projetos"],
            ..base(&projetos)
        };
        assert!(found(&c, &excluded).is_empty());
    }

    #[test]
    fn drive_and_folders_pick_the_volumes() {
        let c = drive_c();
        let mut d = VolumeIndex::new("D:");
        d.upsert(20, ROOT, "relatorio.txt", FILE);
        d.upsert(21, ROOT, "notes.txt", FILE);
        let on = |text: &str| -> Vec<String> {
            let query = Query::parse_for(text, true).unwrap();
            search(&[&c, &d], &base(&query), &|| false)
                .unwrap()
                .hits
                .into_iter()
                .map(|hit| hit.path)
                .collect()
        };
        assert_eq!(on("d:/relatorio"), [r"D:\relatorio.txt"]);
        assert_eq!(on(r"C:\relatorio").len(), 3);
        assert_eq!(
            on("documents/notes"),
            [r"C:\Users\luis\Documents\notes.txt"]
        );
    }

    #[test]
    fn cancelled_folder_search_returns_none() {
        let c = drive_c();
        let query = Query::parse("projetos/").unwrap();
        assert!(search(&[&c], &base(&query), &|| true).is_none());
    }

    #[test]
    fn unix_volumes_use_slashes() {
        let mut root = VolumeIndex::new("/");
        root.upsert(20, ROOT, "home", DIR);
        root.upsert(21, 20, "luis", DIR);
        root.upsert(22, 21, "relatorio.txt", FILE);
        let c = drive_c();
        let outcome = run(
            &[&c, &root],
            "relatorio.txt",
            Filter::Files,
            10,
            Some("/home/luis"),
            false,
        );
        assert_eq!(paths(&outcome), ["/home/luis/relatorio.txt"]);
    }

    #[test]
    fn unix_volumes_answer_slash_queries_without_apps() {
        let mut root = VolumeIndex::new("/");
        root.upsert(20, ROOT, "home", DIR);
        root.upsert(21, 20, "luis", DIR);
        root.upsert(22, 21, "relatorio.txt", FILE);
        root.upsert(23, 21, "tool.exe", FILE);
        let query = Query::parse_for("/home/luis/relatorio", false).unwrap();
        assert_eq!(found(&root, &base(&query)), ["/home/luis/relatorio.txt"]);
        let tool = Query::parse_for("tool", false).unwrap();
        let outcome = search(&[&root], &base(&tool), &|| false).unwrap();
        assert_eq!(paths(&outcome), ["/home/luis/tool.exe"]);
        assert!(!outcome.hits[0].is_app);
    }

    fn catalog() -> AppCatalog {
        let mut catalog = AppCatalog::new();
        catalog.set_system(vec![
            DesktopApp::parse(
                "relatorio.desktop",
                "/usr/share/applications/relatorio.desktop",
                "[Desktop Entry]\nType=Application\nName=Relatorio Studio\nName[pt_BR]=Estúdio de Relatórios\n",
            )
            .unwrap(),
        ]);
        catalog
    }

    fn unix_root() -> VolumeIndex {
        let mut root = VolumeIndex::new("/");
        root.upsert(20, ROOT, "home", DIR);
        root.upsert(21, 20, "luis", DIR);
        root.upsert(22, 21, "relatorio.txt", FILE);
        root.upsert(23, 21, "relatorio", DIR);
        root
    }

    fn app_count(volume: &VolumeIndex, request: &SearchRequest) -> usize {
        search(&[volume], request, &|| false)
            .unwrap()
            .hits
            .iter()
            .filter(|hit| hit.is_app)
            .count()
    }

    #[test]
    fn catalog_apps_come_first_with_their_label() {
        let root = unix_root();
        let apps = catalog();
        let query = Query::parse_for("relatorio", false).unwrap();
        let request = SearchRequest {
            apps: Some(&apps),
            language: "pt-BR",
            ..base(&query)
        };
        let outcome = search(&[&root], &request, &|| false).unwrap();
        assert_eq!(
            paths(&outcome),
            [
                "/usr/share/applications/relatorio.desktop",
                "/home/luis/relatorio",
                "/home/luis/relatorio.txt",
            ]
        );
        assert_eq!(
            outcome.hits[0].label.as_deref(),
            Some("Estúdio de Relatórios")
        );
        assert!(outcome.hits[0].is_app);
        assert_eq!(outcome.hits[1].label, None);
        assert_eq!(outcome.total, 3);
        let one = SearchRequest {
            limit: 1,
            ..request
        };
        let outcome = search(&[&root], &one, &|| false).unwrap();
        assert_eq!(
            paths(&outcome),
            ["/usr/share/applications/relatorio.desktop"]
        );
        assert_eq!(outcome.total, 3);
    }

    #[test]
    fn catalog_apps_only_answer_app_searches() {
        let root = unix_root();
        let apps = catalog();
        let query = Query::parse_for("relatorio", false).unwrap();
        let only_apps = SearchRequest {
            apps: Some(&apps),
            filter: Filter::Apps,
            ..base(&query)
        };
        assert_eq!(app_count(&root, &only_apps), 1);
        let files = SearchRequest {
            apps: Some(&apps),
            filter: Filter::Files,
            ..base(&query)
        };
        assert_eq!(app_count(&root, &files), 0);
        let no_apps = SearchRequest {
            apps: Some(&apps),
            categories: Categories {
                apps: false,
                ..Categories::ALL
            },
            ..base(&query)
        };
        assert_eq!(app_count(&root, &no_apps), 0);
        let folder = Query::parse_for("luis/relatorio", false).unwrap();
        let in_folder = SearchRequest {
            apps: Some(&apps),
            ..base(&folder)
        };
        assert_eq!(app_count(&root, &in_folder), 0);
    }
}
