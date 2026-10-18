pub mod apps;
pub mod disk;
pub mod keyboard;
pub mod runner;
pub mod socket;
pub mod watch;

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crate::engine::Engine;
use crate::log::{self, Logger, log};
use keyboard::Keyboard;
use runner::Runner;
use watch::Watcher;

/// `/var/lib/tex`, or `TEX_INDEXER_DATA` in debug builds (to run the console without root).
pub fn data_dir() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(dir) = std::env::var_os("TEX_INDEXER_DATA")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    PathBuf::from("/var/lib/tex")
}

/// Creates the private data folder, starts the log, reads the apps, begins indexing every
/// local file system and creates the virtual keyboard for automatic paste (root only).
pub fn start(echo_log: bool) -> io::Result<Arc<Runner>> {
    let data = data_dir();
    fs::create_dir_all(&data)?;
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
    let index_dir = data.join("index");
    fs::create_dir_all(&index_dir)?;
    log::init(Logger::new(data.join("logs").join("indexer.log"), echo_log));
    let watcher = match Watcher::start() {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log(format!("live updates unavailable ({error})"));
            None
        }
    };
    let keyboard = match Keyboard::create() {
        Ok(keyboard) => {
            log("virtual keyboard ready for automatic paste");
            Some(keyboard)
        }
        Err(error) => {
            log(format!("automatic paste unavailable ({error})"));
            None
        }
    };
    let runner = Runner::new(Arc::new(Engine::new()), index_dir, watcher, keyboard);
    runner.reload_apps();
    runner.discover();
    Ok(runner)
}

/// Blocks SIGINT and SIGTERM in this thread and in the threads it starts later, and calls
/// `on_stop` on a thread of its own when one of them arrives. Call before starting threads.
pub fn on_stop_signal(on_stop: impl FnOnce() + Send + 'static) {
    let mut signals: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut signals);
        libc::sigaddset(&mut signals, libc::SIGINT);
        libc::sigaddset(&mut signals, libc::SIGTERM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut());
    }
    thread::spawn(move || {
        let mut signal = 0;
        unsafe { libc::sigwait(&signals, &mut signal) };
        on_stop();
    });
}
