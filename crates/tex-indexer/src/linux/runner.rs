use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use tex_index::{DenseIds, Record, SnapshotError, SnapshotHeader, Viewer, VolumeIndex};
use tex_ipc::{Request, Response, VolumeState};

use crate::desktop::{SYSTEM_APP_DIRS, is_app_folder, user_app_dirs};
use crate::engine::{Engine, Handler, Volume};
use crate::events::ITSELF;
use crate::linux::apps::{home_of, read_apps};
use crate::linux::disk;
use crate::linux::watch::{Batch, Watcher};
use crate::log::log;
use crate::mounts::{Mount, boundaries, indexable, parse_mountinfo, volume_key};
use crate::store;
use crate::tree::{self, Node};

const SAVE_EVERY: Duration = Duration::from_secs(10 * 60);

/// Indexes every local file system: a thread per volume loads its snapshot, scans it and then
/// applies the watcher's changes.
pub struct Runner {
    engine: Arc<Engine>,
    index_dir: PathBuf,
    watcher: Option<Watcher>,
    homes: Mutex<HashMap<u32, String>>,
    me: Weak<Runner>,
}

impl Runner {
    pub fn new(engine: Arc<Engine>, index_dir: PathBuf, watcher: Option<Watcher>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            engine,
            index_dir,
            watcher,
            homes: Mutex::new(HashMap::new()),
            me: me.clone(),
        })
    }

    /// Starts a thread for each indexable file system not tracked yet (or tracked but failed)
    /// and forgets volumes that are no longer mounted.
    pub fn discover(&self) {
        let Some(runner) = self.me.upgrade() else {
            return;
        };
        let Ok(text) = fs::read_to_string("/proc/self/mountinfo") else {
            return;
        };
        let mounts = parse_mountinfo(&text);
        let wanted = indexable(&mounts);
        for mount in &wanted {
            let Ok((fsid, _)) = disk::file_system(Path::new(&mount.point)) else {
                continue;
            };
            let key = volume_key(fsid, &mount.root);
            if let Some(volume) = self.engine.add(&mount.point, key) {
                log(format!("{} tracking file system {key:016X}", mount.point));
                let stops = boundaries(&mounts, &mount.point);
                let runner = Arc::clone(&runner);
                thread::spawn(move || runner.run_volume(&volume, fsid, &stops));
            }
        }
        for volume in self.engine.volumes() {
            if !wanted
                .iter()
                .any(|mount: &Mount| mount.point == volume.root)
            {
                log(format!("{} is no longer mounted", volume.root));
                self.engine.remove(&volume.root);
                if let Some(watcher) = &self.watcher {
                    watcher.unroute(&volume.root);
                }
            }
        }
    }

    pub fn save_all(&self) {
        for volume in self.engine.volumes() {
            self.save(&volume);
        }
    }

    /// Reads the system's `.desktop` apps and those of every user seen so far.
    pub fn reload_apps(&self) {
        let system = read_apps(&SYSTEM_APP_DIRS.map(String::from));
        let homes: Vec<String> = self
            .homes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        let users: Vec<(String, _)> = homes
            .into_iter()
            .map(|home| {
                let apps = read_apps(&user_app_dirs(&home));
                (home, apps)
            })
            .collect();
        let mut catalog = self.engine.apps_mut();
        catalog.set_system(system);
        for (home, apps) in users {
            catalog.set_user(&home, apps);
        }
    }

    /// The home of `uid`, reading that user's apps the first time.
    fn home(&self, uid: u32) -> Option<String> {
        if let Some(home) = self
            .homes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uid)
        {
            return Some(home.clone());
        }
        let home = home_of(uid)?;
        self.homes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uid, home.clone());
        let apps = read_apps(&user_app_dirs(&home));
        self.engine.apps_mut().set_user(&home, apps);
        Some(home)
    }

    /// The mount points under `point`, read again so that file systems mounted since the
    /// volume started are not walked as part of it.
    fn stops(point: &str) -> HashSet<PathBuf> {
        fs::read_to_string("/proc/self/mountinfo")
            .map(|text| boundaries(&parse_mountinfo(&text), point))
            .unwrap_or_default()
    }

    fn run_volume(&self, volume: &Volume, fsid: u64, stops: &HashSet<PathBuf>) {
        let mut stops = stops.clone();
        let (sender, receiver) = mpsc::channel();
        let live = match &self.watcher {
            Some(watcher) => {
                watcher.route(fsid, &volume.root, sender);
                match watcher.mark(Path::new(&volume.root)) {
                    Ok(()) => true,
                    Err(error) => {
                        log(format!("{} without live updates ({error})", volume.root));
                        false
                    }
                }
            }
            None => false,
        };
        if let Some(index) = self.load(volume) {
            log(format!(
                "{} serving the snapshot with {} entries",
                volume.root,
                index.len()
            ));
            let mut data = volume.write();
            data.state = VolumeState::Ready {
                entries: index.len() as u32,
            };
            data.index = Some(index);
        }
        let mut ids = self.scan(volume, &stops);
        if !live {
            return;
        }
        let mut last_saved = Instant::now();
        while let Ok(batch) = receiver.recv() {
            match batch {
                Batch::Rescan => {
                    log(format!("{} reindexing (events were dropped)", volume.root));
                    stops = Self::stops(&volume.root);
                    ids = self.scan(volume, &stops);
                }
                Batch::Changes(changes) => self.apply(volume, &mut ids, &stops, changes),
            }
            if last_saved.elapsed() >= SAVE_EVERY {
                self.save(volume);
                last_saved = Instant::now();
            }
        }
    }

    /// Builds a new index of the volume and puts it in place of the current one.
    fn scan(&self, volume: &Volume, stops: &HashSet<PathBuf>) -> DenseIds {
        log(format!("{} indexing", volume.root));
        let total = disk::file_system(Path::new(&volume.root))
            .map_or(0, |(_, used)| used)
            .max(1);
        let quiet = volume.read().index.is_some();
        let mut shown = 0;
        let (index, ids) = disk::scan(&volume.root, stops, &mut |added| {
            let percent = (added as u64 * 100 / total).min(99) as u8;
            if !quiet && percent != shown {
                shown = percent;
                volume.write().state = VolumeState::Indexing { percent };
            }
        });
        log(format!(
            "{} ready with {} entries",
            volume.root,
            index.len()
        ));
        {
            let mut data = volume.write();
            data.state = VolumeState::Ready {
                entries: index.len() as u32,
            };
            data.index = Some(index);
        }
        self.save(volume);
        ids
    }

    /// Looks again at each changed entry and updates the index; folders that appear are walked.
    fn apply(
        &self,
        volume: &Volume,
        ids: &mut DenseIds,
        stops: &HashSet<PathBuf>,
        changes: Vec<(Vec<u8>, String)>,
    ) {
        let mut seen = HashSet::new();
        let mut targets: Vec<(Record, String, PathBuf)> = Vec::new();
        let mut itself: Vec<(Record, PathBuf)> = Vec::new();
        {
            let data = volume.read();
            let Some(index) = data.index.as_ref() else {
                return;
            };
            for (dir, name) in changes
                .into_iter()
                .filter(|change| seen.insert(change.clone()))
            {
                let Some(record) = ids.dir(&dir) else {
                    continue;
                };
                let Some(folder) = index.path(record) else {
                    continue;
                };
                if name == ITSELF {
                    itself.push((record, PathBuf::from(folder)));
                } else {
                    let path = PathBuf::from(folder).join(&name);
                    targets.push((record, name, path));
                }
            }
        }
        let folders: Vec<(Record, Option<Node>)> = itself
            .into_iter()
            .map(|(record, path)| (record, disk::probe(&path)))
            .filter(|(_, node)| node.as_ref().is_some_and(|node| node.dir))
            .collect();
        if targets.is_empty() && folders.is_empty() {
            return;
        }
        let mut found: Vec<((Record, String, PathBuf), Option<Node>)> = targets
            .into_iter()
            .map(|target| {
                let node = disk::probe(&target.2);
                (target, node)
            })
            .collect();
        found.sort_by_key(|(_, node)| node.is_none());
        let apps_changed = found.iter().any(|((_, _, path), _)| {
            path.parent()
                .and_then(Path::to_str)
                .is_some_and(is_app_folder)
        });
        {
            let mut guard = volume.write();
            let data = &mut *guard;
            let Some(index) = data.index.as_mut() else {
                return;
            };
            for (record, node) in folders {
                if let Some(node) = node {
                    index.set_owner(record, node.owner);
                }
            }
            let mut fresh = Vec::new();
            for ((parent, name, path), mut node) in found {
                let crossing = stops.contains(&path);
                if crossing && let Some(node) = node.as_mut() {
                    node.handle = None;
                }
                if let Some(record) = tree::reconcile(index, ids, parent, &name, node)
                    && !crossing
                {
                    fresh.push((record, path));
                }
            }
            for (record, path) in fresh {
                tree::walk(
                    index,
                    ids,
                    record,
                    &path,
                    stops,
                    &mut disk::list,
                    &mut |_| {},
                );
            }
            data.state = VolumeState::Ready {
                entries: index.len() as u32,
            };
        }
        if apps_changed {
            self.reload_apps();
        }
    }

    fn load(&self, volume: &Volume) -> Option<VolumeIndex> {
        match store::load(&self.index_dir, volume.serial) {
            Ok((index, header))
                if header.serial == volume.serial && index.root() == volume.root =>
            {
                Some(index)
            }
            Ok(_) => {
                log(format!("{} snapshot is of another volume", volume.root));
                None
            }
            Err(SnapshotError::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                log(format!("{} ignoring snapshot ({error})", volume.root));
                None
            }
        }
    }

    fn save(&self, volume: &Volume) {
        let data = volume.read();
        let Some(index) = data.index.as_ref() else {
            return;
        };
        let header = SnapshotHeader {
            serial: volume.serial,
            journal_id: 0,
            next_usn: 0,
        };
        if let Err(error) = store::save(&self.index_dir, index, &header) {
            log(format!("{} could not save snapshot ({error})", volume.root));
        }
    }
}

impl Handler for Runner {
    fn handle(&self, request: Request) -> Response {
        self.handle_as(request, None)
    }

    /// Answers as `viewer`, with that user's home (from the user database) and apps.
    fn handle_as(&self, request: Request, viewer: Option<&Viewer>) -> Response {
        match request {
            Request::Status => {
                self.discover();
                self.engine.handle_as(Request::Status, viewer)
            }
            Request::Search(mut query) => {
                if let Some(home) = self.home(viewer.map_or(0, |viewer| viewer.uid)) {
                    query.home = home;
                }
                self.engine.handle_as(Request::Search(query), viewer)
            }
        }
    }
}
