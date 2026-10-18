use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::Engine;
use crate::log::{self, Logger};
use crate::runner::Runner;
use crate::security;

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()))
        .join("Tex")
}

/// Creates the private data folder, starts the log and begins indexing every NTFS volume.
pub fn start(echo_log: bool) -> io::Result<Arc<Runner>> {
    let data = data_dir();
    security::create_private_dir(&data)?;
    let index_dir = data.join("index");
    fs::create_dir_all(&index_dir)?;
    log::init(Logger::new(data.join("logs").join("indexer.log"), echo_log));
    let runner = Runner::new(Arc::new(Engine::new()), index_dir);
    runner.discover();
    Ok(runner)
}
