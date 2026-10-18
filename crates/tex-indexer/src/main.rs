#[cfg(windows)]
mod cli_windows;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod service;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    cli_windows::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("tex-indexer: not available on this system yet");
    ExitCode::FAILURE
}
