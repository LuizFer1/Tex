use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crate::entry::{ROOT, Record};
use crate::matcher::{MatchKind, Query};
use crate::normalize::fold_into;
use crate::volume::VolumeIndex;

const CHUNK: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Files,
    Folders,
}

pub struct SearchRequest<'a> {
    pub query: &'a Query,
    pub filter: Filter,
    pub limit: usize,
    pub home: Option<&'a str>,
    pub include_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub total: u32,
}

type Key = (Reverse<MatchKind>, bool, u16, u16);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    key: Key,
    name: String,
    volume: usize,
    record: Record,
    is_dir: bool,
}

/// Searches all volumes in parallel and returns the best `limit` hits, best first.
/// Returns `None` if `cancelled` became true before the search finished.
pub fn search(
    volumes: &[&VolumeIndex],
    request: &SearchRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<SearchOutcome> {
    let homes: Vec<Option<Record>> = volumes
        .iter()
        .map(|volume| request.home.and_then(|home| volume.cached_lookup(home)))
        .collect();
    let chunks: Vec<(usize, usize)> = volumes
        .iter()
        .enumerate()
        .flat_map(|(v, volume)| {
            (0..volume.entries().len())
                .step_by(CHUNK)
                .map(move |start| (v, start))
        })
        .collect();
    let next = AtomicUsize::new(0);
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(chunks.len().max(1));

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
                        worker.scan(volumes[v], v, start, homes[v]);
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("search worker panicked"))
            .collect()
    });

    let mut total = 0;
    let mut candidates = Vec::new();
    for worker in results {
        let worker = worker?;
        total += worker.total;
        candidates.extend(worker.heap.into_vec());
    }
    candidates.sort();
    candidates.truncate(request.limit);
    let hits = candidates
        .into_iter()
        .filter_map(|c| {
            Some(Hit {
                path: volumes[c.volume].path(c.record)?,
                is_dir: c.is_dir,
            })
        })
        .collect();
    Some(SearchOutcome { hits, total })
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

    fn scan(
        &mut self,
        volume: &VolumeIndex,
        volume_index: usize,
        start: usize,
        home: Option<Record>,
    ) {
        let entries = volume.entries();
        let end = (start + CHUNK).min(entries.len());
        for (offset, entry) in entries[start..end].iter().enumerate() {
            let record = (start + offset) as Record;
            if !entry.in_use() || record == ROOT {
                continue;
            }
            match self.request.filter {
                Filter::Files if entry.is_dir() => continue,
                Filter::Folders if !entry.is_dir() => continue,
                _ => {}
            }
            let name = volume.entry_name(entry);
            let Some(kind) = self.request.query.matches(name, &mut self.scratch) else {
                continue;
            };
            let Some(lineage) = volume.lineage(record, home) else {
                continue;
            };
            if !lineage.visible && !self.request.include_hidden {
                continue;
            }
            self.total += 1;
            let key = (
                Reverse(kind),
                !lineage.in_home,
                name.len() as u16,
                lineage.depth,
            );
            self.offer(key, name, volume_index, record, entry.is_dir());
        }
    }

    fn offer(&mut self, key: Key, name: &[u8], volume: usize, record: Record, is_dir: bool) {
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
        });
        if self.heap.len() > limit {
            self.heap.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut c = VolumeIndex::new('C');
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
            query: &query,
            filter,
            limit,
            home,
            include_hidden,
        };
        search(volumes, &request, &|| false).unwrap()
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
    fn hidden_items_only_with_include_hidden() {
        let c = drive_c();
        assert_eq!(
            run(&[&c], "relatorio", Filter::All, 10, None, false).total,
            3
        );
        assert_eq!(
            run(&[&c], "relatorio", Filter::All, 10, None, true).total,
            6
        );
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
        let mut d = VolumeIndex::new('D');
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
        let request = SearchRequest {
            query: &query,
            filter: Filter::All,
            limit: 10,
            home: None,
            include_hidden: false,
        };
        assert!(search(&[&c], &request, &|| true).is_none());
    }
}
