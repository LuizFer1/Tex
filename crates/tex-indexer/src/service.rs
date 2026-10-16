use std::error::Error;
use std::ffi::OsString;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use tex_indexer::log::log;
use tex_indexer::pipe;
use tex_ipc::PIPE_NAME;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

pub const NAME: &str = "tex-indexer";

define_windows_service!(ffi_service_main, service_main);

pub fn run() -> Result<(), Box<dyn Error>> {
    service_dispatcher::start(NAME, ffi_service_main)?;
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        log(format!("service failed: {error}"));
    }
}

fn run_service() -> Result<(), Box<dyn Error>> {
    let sid = allowed_sid().ok_or("missing --allow <sid> argument")?;
    let (stop, stopped) = mpsc::channel();
    let status = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    status.set_service_status(service_status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
    ))?;
    let runner = tex_indexer::start(false)?;
    let handler = Arc::clone(&runner);
    thread::spawn(move || {
        if let Err(error) = pipe::serve(PIPE_NAME, &pipe::service_sddl(&sid), handler) {
            log(format!("pipe server stopped: {error}"));
        }
    });
    let _ = stopped.recv();
    status.set_service_status(service_status(
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
    ))?;
    runner.save_all();
    status.set_service_status(service_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
    ))?;
    Ok(())
}

fn service_status(state: ServiceState, accepted: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accepted,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn allowed_sid() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let position = args.iter().position(|arg| arg == "--allow")?;
    args.get(position + 1).cloned()
}
