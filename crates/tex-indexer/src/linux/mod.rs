pub mod apps;
pub mod disk;
pub mod runner;
pub mod socket;
pub mod watch;

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::Engine;
use crate::log::{self, Logger, log};
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

/// Creates the private data folder, starts the log, reads the apps and begins indexing every
/// local file system.
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
    let runner = Runner::new(Arc::new(Engine::new()), index_dir, watcher);
    runner.reload_apps();
    runner.discover();
    Ok(runner)
}
