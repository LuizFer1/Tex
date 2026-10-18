use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use crate::events::{Event, parse_events};
use crate::log::log;

const MASK: u64 = libc::FAN_CREATE
    | libc::FAN_DELETE
    | libc::FAN_MOVED_FROM
    | libc::FAN_MOVED_TO
    | libc::FAN_ATTRIB
    | libc::FAN_ONDIR;

/// What the watcher hands to a volume: entries to look at again (folder handle and name), or
/// `Rescan` after the kernel dropped events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Batch {
    Changes(Vec<(Vec<u8>, String)>),
    Rescan,
}

type RouteMap = HashMap<u64, Vec<(String, Sender<Batch>)>>;
type Routes = Arc<Mutex<RouteMap>>;

/// One fanotify group for every file system, read by its own thread, which sends each file
/// system's changes to the volumes routed to its id.
pub struct Watcher {
    group: File,
    routes: Routes,
}

impl Watcher {
    /// Opens the group and starts its reader thread.
    pub fn start() -> io::Result<Watcher> {
        let fd = unsafe {
            libc::fanotify_init(
                libc::FAN_CLASS_NOTIF | libc::FAN_CLOEXEC | libc::FAN_REPORT_DFID_NAME,
                libc::O_RDONLY as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let group = unsafe { File::from_raw_fd(fd) };
        let reader = group.try_clone()?;
        let routes: Routes = Arc::default();
        let reader_routes = Arc::clone(&routes);
        thread::spawn(move || read_events(reader, &reader_routes));
        Ok(Watcher { group, routes })
    }

    /// Sends the changes of file system `fsid` to the volume at `point` from now on, instead of
    /// to an older thread of that volume.
    pub fn route(&self, fsid: u64, point: &str, to: Sender<Batch>) {
        let mut routes = lock(&self.routes);
        for list in routes.values_mut() {
            list.retain(|(routed, _)| routed != point);
        }
        routes.entry(fsid).or_default().push((point.to_owned(), to));
    }

    /// Stops sending changes to the volume at `point`, which ends its thread.
    pub fn unroute(&self, point: &str) {
        for list in lock(&self.routes).values_mut() {
            list.retain(|(routed, _)| routed != point);
        }
    }

    /// Watches every change on the file system mounted at `point`. Fails where the kernel or
    /// the file system does not allow it (no root, btrfs subvolumes on older kernels, FUSE).
    pub fn mark(&self, point: &Path) -> io::Result<()> {
        let path = CString::new(point.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let result = unsafe {
            libc::fanotify_mark(
                self.group.as_raw_fd(),
                libc::FAN_MARK_ADD | libc::FAN_MARK_FILESYSTEM,
                MASK,
                libc::AT_FDCWD,
                path.as_ptr(),
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn lock(routes: &Routes) -> MutexGuard<'_, RouteMap> {
    routes.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_events(mut group: File, routes: &Routes) {
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = match group.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                log(format!("live updates stopped ({error})"));
                return;
            }
        };
        let mut changes: HashMap<u64, Vec<(Vec<u8>, String)>> = HashMap::new();
        let mut overflow = false;
        for event in parse_events(&buffer[..read]) {
            match event {
                Event::Overflow => overflow = true,
                Event::Change { fsid, dir, name } => {
                    changes.entry(fsid).or_default().push((dir, name));
                }
            }
        }
        let mut routes = lock(routes);
        if overflow {
            for list in routes.values_mut() {
                list.retain(|(_, to)| to.send(Batch::Rescan).is_ok());
            }
        }
        for (fsid, batch) in changes {
            if let Some(list) = routes.get_mut(&fsid) {
                list.retain(|(_, to)| to.send(Batch::Changes(batch.clone())).is_ok());
            }
        }
    }
}
