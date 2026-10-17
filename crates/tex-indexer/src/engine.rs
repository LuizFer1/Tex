use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use tex_index::{Categories, Query, SearchRequest, VolumeIndex};
use tex_ipc::{Filter, Hit, Request, Response, Results, SearchQuery, VolumeState, VolumeStatus};

pub trait Handler: Send + Sync {
    fn handle(&self, request: Request) -> Response;
}

pub struct VolumeData {
    pub index: Option<VolumeIndex>,
    pub state: VolumeState,
    pub journal_id: u64,
    pub next_usn: i64,
}

pub struct Volume {
    pub letter: char,
    pub serial: u32,
    data: RwLock<VolumeData>,
}

impl Volume {
    pub fn new(letter: char, serial: u32) -> Self {
        Self {
            letter,
            serial,
            data: RwLock::new(VolumeData {
                index: None,
                state: VolumeState::Indexing { percent: 0 },
                journal_id: 0,
                next_usn: 0,
            }),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, VolumeData> {
        self.data.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, VolumeData> {
        self.data.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Default)]
pub struct Engine {
    volumes: Mutex<Vec<Arc<Volume>>>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracks a volume, replacing one with the same letter only if it failed or its serial changed.
    /// Returns `None` when the letter is already tracked and healthy.
    pub fn add(&self, letter: char, serial: u32) -> Option<Arc<Volume>> {
        let mut volumes = self.volumes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(position) = volumes.iter().position(|volume| volume.letter == letter) {
            let existing = &volumes[position];
            let failed = matches!(existing.read().state, VolumeState::Failed { .. });
            if !failed && existing.serial == serial {
                return None;
            }
            volumes.remove(position);
        }
        let volume = Arc::new(Volume::new(letter, serial));
        volumes.push(Arc::clone(&volume));
        Some(volume)
    }

    pub fn volumes(&self) -> Vec<Arc<Volume>> {
        self.volumes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn status(&self) -> Vec<VolumeStatus> {
        let mut status: Vec<VolumeStatus> = self
            .volumes()
            .iter()
            .map(|volume| VolumeStatus {
                letter: volume.letter,
                state: volume.read().state.clone(),
            })
            .collect();
        status.sort_by_key(|volume| volume.letter);
        status
    }

    pub fn search(&self, request: &SearchQuery) -> Results {
        let empty = Results {
            id: request.id,
            hits: Vec::new(),
            total: 0,
        };
        let Some(query) = Query::parse(&request.query) else {
            return empty;
        };
        let volumes = self.volumes();
        let guards: Vec<_> = volumes.iter().map(|volume| volume.read()).collect();
        let indexes: Vec<&VolumeIndex> = guards
            .iter()
            .filter_map(|data| data.index.as_ref())
            .collect();
        let filter = match request.filter {
            Filter::All => tex_index::Filter::All,
            Filter::Files => tex_index::Filter::Files,
            Filter::Folders => tex_index::Filter::Folders,
            Filter::Apps => tex_index::Filter::Apps,
        };
        let program_data = std::env::var("ProgramData").ok();
        let roots = start_menu_roots(program_data.as_deref(), &request.home);
        let app_roots: Vec<&str> = roots.iter().map(String::as_str).collect();
        let excluded: Vec<&str> = request
            .excluded_folders
            .iter()
            .map(String::as_str)
            .collect();
        let search_request = SearchRequest {
            query: &query,
            filter,
            limit: request.limit as usize,
            home: (!request.home.is_empty()).then_some(request.home.as_str()),
            include_hidden: request.include_hidden,
            include_system: request.include_system,
            excluded_folders: &excluded,
            categories: Categories {
                apps: request.show_apps,
                files: request.show_files,
                folders: request.show_folders,
            },
            app_roots: &app_roots,
        };
        let Some(outcome) = tex_index::search(&indexes, &search_request, &|| false) else {
            return empty;
        };
        Results {
            id: request.id,
            hits: outcome
                .hits
                .into_iter()
                .map(|hit| Hit {
                    path: hit.path,
                    is_dir: hit.is_dir,
                    is_app: hit.is_app,
                })
                .collect(),
            total: outcome.total,
        }
    }
}

/// Start Menu folders whose shortcuts count as apps: the shared one and the requesting user's.
pub fn start_menu_roots(program_data: Option<&str>, home: &str) -> Vec<String> {
    const PROGRAMS: &str = r"Microsoft\Windows\Start Menu\Programs";
    let mut roots = Vec::new();
    if let Some(program_data) = program_data.filter(|path| !path.is_empty()) {
        roots.push(format!(
            r"{}\{PROGRAMS}",
            program_data.trim_end_matches('\\')
        ));
    }
    if !home.is_empty() {
        roots.push(format!(
            r"{}\AppData\Roaming\{PROGRAMS}",
            home.trim_end_matches('\\')
        ));
    }
    roots
}

impl Handler for Engine {
    fn handle(&self, request: Request) -> Response {
        match request {
            Request::Search(query) => Response::Results(self.search(&query)),
            Request::Status => Response::Status(self.status()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_index::{Attributes, ROOT};
    use tex_ipc::{Filter, Hit, SearchQuery};

    fn ready(engine: &Engine, letter: char, entries: &[(u32, u32, &str, bool)]) -> Arc<Volume> {
        let volume = engine.add(letter, 1).unwrap();
        let mut index = VolumeIndex::new(letter);
        for &(record, parent, name, dir) in entries {
            index.upsert(
                record,
                parent,
                name,
                Attributes {
                    dir,
                    ..Attributes::default()
                },
            );
        }
        let mut data = volume.write();
        data.state = VolumeState::Ready {
            entries: index.len() as u32,
        };
        data.index = Some(index);
        drop(data);
        volume
    }

    fn query(text: &str, filter: Filter) -> SearchQuery {
        SearchQuery {
            id: 7,
            query: text.into(),
            filter,
            limit: 50,
            home: String::new(),
            include_hidden: false,
            include_system: false,
            excluded_folders: Vec::new(),
            show_apps: true,
            show_files: true,
            show_folders: true,
        }
    }

    #[test]
    fn passes_search_options_to_the_index() {
        let engine = Engine::new();
        ready(
            &engine,
            'C',
            &[
                (20, ROOT, "Projetos", true),
                (21, 20, "alpha.txt", false),
                (22, 20, "alpha", true),
                (30, ROOT, "Outros", true),
                (31, 30, "alpha.md", false),
            ],
        );
        let request = SearchQuery {
            excluded_folders: vec![r"C:\Outros".into()],
            show_folders: false,
            ..query("alpha", Filter::All)
        };
        let paths: Vec<String> = engine
            .search(&request)
            .hits
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(paths, [r"C:\Projetos\alpha.txt"]);
    }

    #[test]
    fn searches_ready_volumes_only() {
        let engine = Engine::new();
        ready(
            &engine,
            'C',
            &[
                (20, ROOT, "Projetos", true),
                (21, 20, "relatorio.pdf", false),
            ],
        );
        engine.add('D', 2).unwrap();
        let results = engine.search(&query("relatorio", Filter::All));
        assert_eq!(results.id, 7);
        assert_eq!(results.total, 1);
        assert_eq!(
            results.hits,
            [Hit {
                path: r"C:\Projetos\relatorio.pdf".into(),
                is_dir: false,
                is_app: false,
            }]
        );
    }

    #[test]
    fn maps_filters() {
        let engine = Engine::new();
        ready(
            &engine,
            'C',
            &[(20, ROOT, "notes", true), (21, 20, "notes.txt", false)],
        );
        assert_eq!(
            engine.search(&query("notes", Filter::Folders)).hits.len(),
            1
        );
        assert_eq!(engine.search(&query("notes", Filter::Files)).hits.len(), 1);
        assert_eq!(engine.search(&query("notes", Filter::All)).hits.len(), 2);
    }

    #[test]
    fn searches_by_path() {
        let engine = Engine::new();
        ready(
            &engine,
            'C',
            &[
                (20, ROOT, "Tex", true),
                (21, 20, "target", true),
                (22, 21, "tex.exe", false),
                (23, 21, "tex.exe.config", false),
                (30, ROOT, "Outros", true),
                (31, 30, "outro.exe", false),
            ],
        );
        let paths: Vec<String> = engine
            .search(&query("Tex/.exe", Filter::All))
            .hits
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(paths, [r"C:\Tex\target\tex.exe"]);
    }

    #[test]
    fn blank_query_returns_no_hits() {
        let engine = Engine::new();
        ready(&engine, 'C', &[(20, ROOT, "a", false)]);
        let results = engine.search(&query("   ", Filter::All));
        assert!(results.hits.is_empty());
        assert_eq!(results.total, 0);
    }

    #[test]
    fn status_lists_volumes_by_letter() {
        let engine = Engine::new();
        engine.add('D', 2).unwrap();
        ready(&engine, 'C', &[(20, ROOT, "a", false)]);
        assert_eq!(
            engine.status(),
            [
                VolumeStatus {
                    letter: 'C',
                    state: VolumeState::Ready { entries: 1 }
                },
                VolumeStatus {
                    letter: 'D',
                    state: VolumeState::Indexing { percent: 0 }
                },
            ]
        );
    }

    #[test]
    fn add_replaces_only_failed_or_changed_volumes() {
        let engine = Engine::new();
        let first = engine.add('C', 1).unwrap();
        assert!(engine.add('C', 1).is_none());
        first.write().state = VolumeState::Failed {
            reason: "gone".into(),
        };
        assert!(engine.add('C', 1).is_some());
        assert!(engine.add('C', 9).is_some());
        assert_eq!(engine.volumes().len(), 1);
        assert_eq!(engine.volumes()[0].serial, 9);
    }

    #[test]
    fn handler_routes_requests() {
        let engine = Engine::new();
        ready(&engine, 'C', &[(20, ROOT, "a.txt", false)]);
        match engine.handle(Request::Search(query("a", Filter::All))) {
            Response::Results(results) => assert_eq!(results.total, 1),
            other => panic!("unexpected {other:?}"),
        }
        match engine.handle(Request::Status) {
            Response::Status(volumes) => assert_eq!(volumes.len(), 1),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn start_menu_roots_cover_shared_and_user_menus() {
        assert_eq!(
            start_menu_roots(Some(r"C:\ProgramData"), r"C:\Users\luis"),
            [
                r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
                r"C:\Users\luis\AppData\Roaming\Microsoft\Windows\Start Menu\Programs",
            ]
        );
        assert!(start_menu_roots(None, "").is_empty());
    }

    #[test]
    fn user_start_menu_shortcuts_are_apps() {
        let engine = Engine::new();
        let hidden_dir = Attributes {
            dir: true,
            hidden: true,
            system: false,
        };
        let volume = engine.add('C', 1).unwrap();
        let mut index = VolumeIndex::new('C');
        index.upsert(
            20,
            ROOT,
            "Users",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(
            21,
            20,
            "luis",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(22, 21, "AppData", hidden_dir);
        index.upsert(
            23,
            22,
            "Roaming",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(
            24,
            23,
            "Microsoft",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(
            25,
            24,
            "Windows",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(
            26,
            25,
            "Start Menu",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(
            27,
            26,
            "Programs",
            Attributes {
                dir: true,
                ..Attributes::default()
            },
        );
        index.upsert(28, 27, "Visual Studio Code.lnk", Attributes::default());
        volume.write().index = Some(index);
        let mut request = query("code", Filter::Apps);
        request.home = r"C:\Users\luis".into();
        let results = engine.search(&request);
        assert_eq!(results.hits.len(), 1);
        assert!(results.hits[0].is_app);
        assert!(
            results.hits[0]
                .path
                .ends_with(r"Programs\Visual Studio Code.lnk")
        );
    }
}
