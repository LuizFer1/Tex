use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;

use tex_index::Viewer;
use tex_ipc::{Request, read_message, write_message};

use crate::engine::Handler;

/// Serves `path` until accepting fails, one thread per client, answering each client as the
/// user who connected. Any local user may connect (mode 0666): the answers are filtered. Fails
/// if another process already serves `path`.
pub fn serve(path: &Path, handler: Arc<dyn Handler>) -> io::Result<()> {
    if UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("{} is already served", path.display()),
        ));
    }
    let _ = fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o666))?;
    for stream in listener.incoming() {
        let stream = stream?;
        let handler = Arc::clone(&handler);
        thread::spawn(move || serve_client(&stream, handler.as_ref()));
    }
    Ok(())
}

fn serve_client(stream: &UnixStream, handler: &dyn Handler) {
    let Ok(viewer) = peer(stream) else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut writer = BufWriter::new(stream);
    while let Ok(request) = read_message::<Request>(&mut reader) {
        let response = handler.handle_as(request, viewer.as_ref());
        if write_message(&mut writer, &response).is_err() {
            break;
        }
    }
}

/// The user on the other end of `stream` (`SO_PEERCRED`, `SO_PEERGROUPS`). `None` for root,
/// who sees everything.
pub fn peer(stream: &UnixStream) -> io::Result<Option<Viewer>> {
    let fd = stream.as_raw_fd();
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &mut len,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if credentials.uid == 0 {
        return Ok(None);
    }
    Ok(Some(Viewer {
        uid: credentials.uid,
        groups: groups(fd, credentials.gid),
    }))
}

/// The peer's groups, its primary group first. Only the primary group when the kernel can not
/// tell the others.
fn groups(fd: RawFd, primary: u32) -> Vec<u32> {
    let mut groups = vec![primary];
    let mut list: Vec<libc::gid_t> = vec![0; 64];
    for _ in 0..2 {
        let mut len = (list.len() * size_of::<libc::gid_t>()) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERGROUPS,
                list.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if result == 0 {
            list.truncate(len as usize / size_of::<libc::gid_t>());
            for group in list {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            return groups;
        }
        if io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            break;
        }
        list.resize(len as usize / size_of::<libc::gid_t>() + 1, 0);
    }
    groups
}
