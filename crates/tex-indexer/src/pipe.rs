use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::os::windows::io::FromRawHandle;
use std::ptr;
use std::sync::Arc;
use std::thread;

use tex_ipc::{Request, read_message, write_message};
use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::engine::Handler;
use crate::security::SecurityDescriptor;
use crate::wide;

const BUFFER_BYTES: u32 = 64 * 1024;

/// DACL for the search pipe: full access for SYSTEM and Administrators, read/write for one user.
pub fn service_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{user_sid})")
}

/// Accepts clients on `name` until an error, one thread per connection. Fails if another
/// process already serves this pipe name.
pub fn serve(name: &str, sddl: &str, handler: Arc<dyn Handler>) -> io::Result<()> {
    let descriptor = SecurityDescriptor::from_sddl(sddl)?;
    let attributes = descriptor.attributes();
    let name = wide(name);
    let mut first = true;
    loop {
        let open_mode = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_BYTES,
                BUFFER_BYTES,
                0,
                &attributes,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        first = false;
        let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let file = unsafe { File::from_raw_handle(pipe) };
        if connected {
            let handler = Arc::clone(&handler);
            thread::spawn(move || serve_client(&file, handler.as_ref()));
        }
    }
}

fn serve_client(file: &File, handler: &dyn Handler) {
    let mut reader = BufReader::new(file);
    let mut writer = BufWriter::new(file);
    while let Ok(request) = read_message::<Request>(&mut reader) {
        if write_message(&mut writer, &handler.handle(request)).is_err() {
            break;
        }
    }
}
