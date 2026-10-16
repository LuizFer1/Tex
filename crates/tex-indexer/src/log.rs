use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub struct Logger {
    path: PathBuf,
    echo: bool,
    lock: Mutex<()>,
}

impl Logger {
    pub fn new(path: PathBuf, echo: bool) -> Self {
        Self {
            path,
            echo,
            lock: Mutex::new(()),
        }
    }

    /// Appends one timestamped line, moving the file to `<name>.old.log` once it reaches 1 MB.
    pub fn write(&self, message: &str) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.echo {
            eprintln!("{message}");
        }
        if fs::metadata(&self.path).is_ok_and(|metadata| metadata.len() >= MAX_BYTES) {
            let _ = fs::rename(&self.path, self.path.with_extension("old.log"));
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{seconds} {message}");
        }
    }
}

pub fn init(logger: Logger) {
    let _ = LOGGER.set(logger);
}

pub fn log(message: impl AsRef<str>) {
    if let Some(logger) = LOGGER.get() {
        logger.write(message.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tex-log-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("indexer.log")
    }

    #[test]
    fn appends_lines() {
        let path = temp_file("append");
        let logger = Logger::new(path.clone(), false);
        logger.write("first");
        logger.write("second");
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().ends_with(" first"));
    }

    #[test]
    fn rotates_after_one_megabyte() {
        let path = temp_file("rotate");
        let logger = Logger::new(path.clone(), false);
        let line = "x".repeat(1000);
        for _ in 0..1100 {
            logger.write(&line);
        }
        assert!(path.with_extension("old.log").exists());
        assert!(fs::metadata(&path).unwrap().len() < MAX_BYTES);
    }
}
