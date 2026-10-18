use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::{Arc, mpsc};
use std::thread;

use tex_indexer::linux::{self, socket};
use tex_indexer::log::log;
use tex_indexer::unit::unit_text;

const UNIT_PATH: &str = "/etc/systemd/system/tex-indexer.service";

pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("console") => serve(true),
        Some("service") => serve(false),
        _ => {
            eprintln!("usage: tex-indexer install | uninstall | console");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tex-indexer: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Indexes and serves the socket until SIGINT or SIGTERM, then saves the snapshots. `echo`
/// also prints the log (console mode).
fn serve(echo: bool) -> Result<(), Box<dyn Error>> {
    let (stop, stopped) = mpsc::channel::<Result<(), String>>();
    let signal_stop = stop.clone();
    linux::on_stop_signal(move || {
        let _ = signal_stop.send(Ok(()));
    });
    let runner = linux::start(echo)?;
    let path = tex_ipc::socket_path();
    if let Some(folder) = Path::new(&path).parent() {
        fs::create_dir_all(folder)?;
    }
    log(format!("serving {path}"));
    let handler = Arc::clone(&runner);
    let server_path = path.clone();
    thread::spawn(move || {
        let result = socket::serve(Path::new(&server_path), handler);
        let _ = stop.send(result.map_err(|error| error.to_string()));
    });
    let result = stopped.recv().unwrap_or(Ok(()));
    log("stopping: saving snapshots");
    runner.save_all();
    if result.is_ok() {
        let _ = fs::remove_file(&path);
    }
    Ok(result?)
}

/// Writes the systemd unit for this executable and starts the service. Run with sudo.
fn install() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let exe = exe.to_str().ok_or("the executable path is not UTF-8")?;
    fs::write(UNIT_PATH, unit_text(exe))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", "tex-indexer.service"])?;
    println!("tex-indexer installed and started from {exe}");
    println!("Keep this file in place; reinstall after moving it. Logs: /var/lib/tex/logs");
    Ok(())
}

fn uninstall() -> Result<(), Box<dyn Error>> {
    systemctl(&["disable", "--now", "tex-indexer.service"])?;
    fs::remove_file(UNIT_PATH)?;
    systemctl(&["daemon-reload"])?;
    println!("tex-indexer removed. Snapshots stay in /var/lib/tex/index");
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new("systemctl").args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {} failed ({status})", args.join(" ")).into())
    }
}
