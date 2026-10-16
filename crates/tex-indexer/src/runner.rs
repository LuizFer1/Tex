use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

use tex_index::{SnapshotError, SnapshotHeader, VolumeIndex};
use tex_ipc::{Request, Response, VolumeState};
use windows_sys::Win32::Foundation::{
    ERROR_JOURNAL_DELETE_IN_PROGRESS, ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE,
};

use crate::engine::{Engine, Handler, Volume};
use crate::log::log;
use crate::store;
use crate::usn::{apply, parse_batch};
use crate::volumes::{Journal, VolumeHandle, ntfs_volumes, windows_dir};

const SAVE_EVERY: Duration = Duration::from_secs(10 * 60);
const RECORD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

enum Failure {
    Rescan(String),
    Fatal(io::Error),
}

pub struct Runner {
    engine: Arc<Engine>,
    index_dir: PathBuf,
    windows_dir: Option<String>,
    me: Weak<Runner>,
}

impl Runner {
    pub fn new(engine: Arc<Engine>, index_dir: PathBuf) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            engine,
            index_dir,
            windows_dir: windows_dir(),
            me: me.clone(),
        })
    }

    /// Starts an indexing thread for each NTFS volume not tracked yet, or tracked but failed.
    pub fn discover(&self) {
        let Some(runner) = self.me.upgrade() else {
            return;
        };
        for found in ntfs_volumes() {
            if let Some(volume) = self.engine.add(found.letter, found.serial) {
                log(format!(
                    "{}: tracking volume {:08X}",
                    found.letter, found.serial
                ));
                let runner = Arc::clone(&runner);
                thread::spawn(move || runner.run_volume(&volume));
            }
        }
    }

    pub fn save_all(&self) {
        for volume in self.engine.volumes() {
            self.save(&volume);
        }
    }

    fn run_volume(&self, volume: &Volume) {
        loop {
            match self.follow(volume) {
                Failure::Rescan(reason) => log(format!("{}: reindexing ({reason})", volume.letter)),
                Failure::Fatal(error) => {
                    log(format!("{}: stopped ({error})", volume.letter));
                    let mut data = volume.write();
                    data.index = None;
                    data.state = VolumeState::Failed {
                        reason: error.to_string(),
                    };
                    return;
                }
            }
        }
    }

    /// Loads the snapshot or builds the index, then applies journal records until something fails.
    fn follow(&self, volume: &Volume) -> Failure {
        let handle = match VolumeHandle::open(volume.letter) {
            Ok(handle) => handle,
            Err(error) => return Failure::Fatal(error),
        };
        let journal = match handle.journal() {
            Ok(journal) => journal,
            Err(error) => return Failure::Fatal(error),
        };
        let (index, mut next_usn, fresh) = match self.load(volume, &journal) {
            Some((index, next_usn)) => (index, next_usn, false),
            None => match self.build(volume, &handle, &journal) {
                Ok(index) => (index, journal.next_usn, true),
                Err(error) => return Failure::Fatal(error),
            },
        };
        log(format!(
            "{}: ready with {} entries",
            volume.letter,
            index.len()
        ));
        {
            let mut data = volume.write();
            data.state = VolumeState::Ready {
                entries: index.len() as u32,
            };
            data.index = Some(index);
            data.journal_id = journal.id;
            data.next_usn = next_usn;
        }
        if fresh {
            self.save(volume);
        }
        let mut last_saved = Instant::now();
        let mut buffer = vec![0u64; 64 * 1024];
        loop {
            let bytes = match handle.read_journal(journal.id, next_usn, &mut buffer) {
                Ok(bytes) => bytes,
                Err(error) => return classify(error),
            };
            let Some((next, records)) = parse_batch(bytes) else {
                continue;
            };
            next_usn = next as i64;
            {
                let mut guard = volume.write();
                let data = &mut *guard;
                if let Some(index) = data.index.as_mut() {
                    for record in &records {
                        apply(index, record);
                    }
                    data.state = VolumeState::Ready {
                        entries: index.len() as u32,
                    };
                }
                data.next_usn = next_usn;
            }
            if last_saved.elapsed() >= SAVE_EVERY {
                self.save(volume);
                last_saved = Instant::now();
            }
        }
    }

    fn load(&self, volume: &Volume, journal: &Journal) -> Option<(VolumeIndex, i64)> {
        match store::load(&self.index_dir, volume.serial) {
            Ok((index, header))
                if header.serial == volume.serial
                    && header.journal_id == journal.id
                    && (journal.first_usn..=journal.next_usn).contains(&header.next_usn)
                    && index.letter() == volume.letter =>
            {
                Some((index, header.next_usn))
            }
            Ok(_) => {
                log(format!("{}: snapshot is out of date", volume.letter));
                None
            }
            Err(SnapshotError::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                log(format!("{}: ignoring snapshot ({error})", volume.letter));
                None
            }
        }
    }

    fn build(
        &self,
        volume: &Volume,
        handle: &VolumeHandle,
        journal: &Journal,
    ) -> io::Result<VolumeIndex> {
        log(format!("{}: indexing", volume.letter));
        volume.write().state = VolumeState::Indexing { percent: 0 };
        let total = handle.record_count().unwrap_or(0).max(1);
        let mut index = VolumeIndex::new(volume.letter);
        handle.enumerate(journal.next_usn, |bytes| {
            let Some((next, records)) = parse_batch(bytes) else {
                return;
            };
            for record in &records {
                index.upsert(
                    record.record,
                    record.parent,
                    &record.name,
                    record.attributes,
                );
            }
            let percent = ((next & RECORD_MASK) * 100 / total).min(99) as u8;
            volume.write().state = VolumeState::Indexing { percent };
        })?;
        if let Some(dir) = &self.windows_dir {
            index.set_windows_dir(index.lookup(dir));
        }
        Ok(index)
    }

    fn save(&self, volume: &Volume) {
        let data = volume.read();
        let Some(index) = data.index.as_ref() else {
            return;
        };
        let header = SnapshotHeader {
            serial: volume.serial,
            journal_id: data.journal_id,
            next_usn: data.next_usn,
        };
        if let Err(error) = store::save(&self.index_dir, index, &header) {
            log(format!(
                "{}: could not save snapshot ({error})",
                volume.letter
            ));
        }
    }
}

fn classify(error: io::Error) -> Failure {
    match error.raw_os_error().map(|code| code as u32) {
        Some(
            ERROR_JOURNAL_ENTRY_DELETED
            | ERROR_JOURNAL_DELETE_IN_PROGRESS
            | ERROR_JOURNAL_NOT_ACTIVE,
        ) => Failure::Rescan(error.to_string()),
        _ => Failure::Fatal(error),
    }
}

impl Handler for Runner {
    fn handle(&self, request: Request) -> Response {
        if matches!(request, Request::Status) {
            self.discover();
        }
        self.engine.handle(request)
    }
}
