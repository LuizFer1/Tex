use std::error::Error;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;

use tex_indexer::pipe::PipeAccess;
use tex_indexer::{account, log, pipe};
use tex_ipc::PIPE_NAME;
use windows_sys::Win32::Foundation::BOOL;
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

use crate::{install, service};

static CONSOLE_STOP: OnceLock<Sender<Result<(), String>>> = OnceLock::new();

pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("install") => install::install(args[1..].iter().any(|arg| arg == "--all-users")),
        Some("uninstall") => install::uninstall(),
        Some("console") => console(),
        Some("service") => service::run(),
        _ => {
            eprintln!("usage: tex-indexer install [--all-users] | uninstall | console");
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

/// Serves in the foreground until Ctrl+C or the console closes, then saves the snapshots
/// like the service does when it stops.
fn console() -> Result<(), Box<dyn Error>> {
    let sid = account::current_user_sid()?;
    let runner = tex_indexer::start(true)?;
    log::log(format!("console mode: serving {PIPE_NAME} for {sid}"));
    let (stop, stopped) = mpsc::channel();
    let pipe_stop = stop.clone();
    let _ = CONSOLE_STOP.set(stop);
    unsafe { SetConsoleCtrlHandler(Some(on_console_ctrl), 1) };
    let handler = Arc::clone(&runner);
    let sddl = pipe::service_sddl(&PipeAccess::User(sid));
    thread::spawn(move || {
        let result = pipe::serve(PIPE_NAME, &sddl, handler).map_err(|error| error.to_string());
        let _ = pipe_stop.send(result);
    });
    let result = stopped.recv().unwrap_or(Ok(()));
    log::log("console mode: saving snapshots");
    runner.save_all();
    Ok(result?)
}

unsafe extern "system" fn on_console_ctrl(_event: u32) -> BOOL {
    if let Some(stop) = CONSOLE_STOP.get() {
        let _ = stop.send(Ok(()));
    }
    loop {
        thread::park();
    }
}
