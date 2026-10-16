use std::error::Error;
use std::thread;
use std::time::Duration;

use tex_indexer::account;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::service::NAME;

/// Registers the service for the current user and starts it. Run from an administrator terminal.
pub fn install() -> Result<(), Box<dyn Error>> {
    let sid = account::current_user_sid()?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: "Tex Indexer".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec!["service".into(), "--allow".into(), sid.into()],
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let service =
        manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    service.set_description("Indexes NTFS drives for Tex search")?;
    service.start::<&str>(&[])?;
    println!(
        "tex-indexer installed and started from {}",
        info.executable_path.display()
    );
    println!("Keep this file in place; reinstall after moving it. Logs: %ProgramData%\\Tex\\logs");
    Ok(())
}

pub fn uninstall() -> Result<(), Box<dyn Error>> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    )?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
    }
    for _ in 0..50 {
        if service.query_status()?.current_state == ServiceState::Stopped {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    service.delete()?;
    println!("tex-indexer removed. Snapshots stay in %ProgramData%\\Tex\\index");
    Ok(())
}
