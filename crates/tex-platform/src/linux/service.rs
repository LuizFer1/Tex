use std::io;

use zbus::blocking::object_server::InterfaceRef;
use zbus::fdo::RequestNameFlags;
use zbus::interface;

use super::dbus::session_bus;
use crate::AppCommand;

pub(crate) const NAME: &str = "io.github.LuizFer1.Tex";
const PATH: &str = "/io/github/LuizFer1/Tex";

type OnCommand = Box<dyn Fn(AppCommand) + Send + Sync>;

struct App {
    on_command: OnCommand,
    paused: bool,
    update_ready: String,
    labels: Vec<String>,
}

#[interface(name = "io.github.LuizFer1.Tex")]
impl App {
    fn toggle(&self) {
        (self.on_command)(AppCommand::Toggle);
    }

    fn history(&self) {
        (self.on_command)(AppCommand::History);
    }

    fn settings(&self) {
        (self.on_command)(AppCommand::Settings);
    }

    fn about(&self) {
        (self.on_command)(AppCommand::About);
    }

    fn toggle_pause(&self) {
        (self.on_command)(AppCommand::TogglePause);
    }

    fn apply_update(&self) {
        (self.on_command)(AppCommand::ApplyUpdate);
    }

    fn quit(&self) {
        (self.on_command)(AppCommand::Quit);
    }

    #[zbus(property)]
    fn paused(&self) -> bool {
        self.paused
    }

    #[zbus(property)]
    fn update_ready(&self) -> String {
        self.update_ready.clone()
    }

    /// The menu texts in the user's language: open, history, settings, about, pause or
    /// resume, update, quit.
    #[zbus(property)]
    fn labels(&self) -> Vec<String> {
        self.labels.clone()
    }
}

/// Tex's object on the session bus (`io.github.LuizFer1.Tex`), which the GNOME extension and
/// `tex --toggle` call. The name goes away with the process.
pub struct AppBus {
    app: InterfaceRef<App>,
}

impl AppBus {
    /// Takes the name and serves the commands, which `on_command` receives on a bus thread.
    /// `None` without a session bus or when another process has the name.
    pub fn start(on_command: impl Fn(AppCommand) + Send + Sync + 'static) -> Option<AppBus> {
        let bus = session_bus()?;
        let app = App {
            on_command: Box::new(on_command),
            paused: false,
            update_ready: String::new(),
            labels: Vec::new(),
        };
        bus.object_server().at(PATH, app).ok()?;
        bus.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into())
            .ok()?;
        let app = bus.object_server().interface::<_, App>(PATH).ok()?;
        Some(AppBus { app })
    }

    pub fn set_paused(&self, paused: bool) {
        let mut app = self.app.get_mut();
        if app.paused != paused {
            app.paused = paused;
            let _ = zbus::block_on(app.paused_changed(self.app.signal_emitter()));
        }
    }

    pub fn set_update_ready(&self, version: &str) {
        let mut app = self.app.get_mut();
        if app.update_ready != version {
            app.update_ready = version.to_owned();
            let _ = zbus::block_on(app.update_ready_changed(self.app.signal_emitter()));
        }
    }

    pub fn set_labels(&self, labels: Vec<String>) {
        let mut app = self.app.get_mut();
        if app.labels != labels {
            app.labels = labels;
            let _ = zbus::block_on(app.labels_changed(self.app.signal_emitter()));
        }
    }
}

/// Sends `command` to the Tex that holds the name on the session bus.
pub fn send_command(command: AppCommand) -> io::Result<()> {
    let bus = session_bus()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no session bus"))?;
    bus.call_method(Some(NAME), PATH, Some(NAME), method(command), &())
        .map(drop)
        .map_err(io::Error::other)
}

pub(crate) fn method(command: AppCommand) -> &'static str {
    match command {
        AppCommand::Toggle => "Toggle",
        AppCommand::History => "History",
        AppCommand::Settings => "Settings",
        AppCommand::About => "About",
        AppCommand::TogglePause => "TogglePause",
        AppCommand::ApplyUpdate => "ApplyUpdate",
        AppCommand::Quit => "Quit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_its_method() {
        assert_eq!(method(AppCommand::Toggle), "Toggle");
        assert_eq!(method(AppCommand::TogglePause), "TogglePause");
        assert_eq!(method(AppCommand::ApplyUpdate), "ApplyUpdate");
        assert_eq!(method(AppCommand::Quit), "Quit");
    }
}
