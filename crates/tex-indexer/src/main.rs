mod install;
mod service;

use std::error::Error;
use std::process::ExitCode;

use tex_indexer::{account, log, pipe};
use tex_ipc::PIPE_NAME;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("install") => install::install(),
        Some("uninstall") => install::uninstall(),
        Some("console") => console(),
        Some("service") => service::run(),
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

fn console() -> Result<(), Box<dyn Error>> {
    let sid = account::current_user_sid()?;
    let runner = tex_indexer::start(true)?;
    log::log(format!("console mode: serving {PIPE_NAME} for {sid}"));
    pipe::serve(PIPE_NAME, &pipe::service_sddl(&sid), runner)?;
    Ok(())
}
