use std::ffi::OsString;
use std::sync::OnceLock;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::WorkArea;

/// A rectangle in physical pixels of the X screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}

/// One RandR monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Monitor {
    pub rect: Rect,
    pub primary: bool,
    pub width_mm: u32,
    pub height_mm: u32,
}

struct Display {
    connection: RustConnection,
    root: Window,
    screen: usize,
}

/// The X server of this session, opened once. `None` on Wayland, even with XWayland present,
/// because the app's windows are then Wayland windows.
fn display() -> Option<&'static Display> {
    static DISPLAY: OnceLock<Option<Display>> = OnceLock::new();
    DISPLAY
        .get_or_init(|| {
            if !is_x11_session(
                std::env::var_os("WAYLAND_DISPLAY"),
                std::env::var_os("WAYLAND_SOCKET"),
                std::env::var_os("DISPLAY"),
            ) {
                return None;
            }
            let (connection, screen) = x11rb::connect(None).ok()?;
            let root = connection.setup().roots.get(screen)?.root;
            Some(Display {
                connection,
                root,
                screen,
            })
        })
        .as_ref()
}

/// Whether winit runs on X11: it picks Wayland whenever `WAYLAND_DISPLAY` or `WAYLAND_SOCKET`
/// is set and not empty, and X11 when only `DISPLAY` is.
pub(crate) fn is_x11_session(
    wayland_display: Option<OsString>,
    wayland_socket: Option<OsString>,
    display: Option<OsString>,
) -> bool {
    let set = |value: &Option<OsString>| value.as_ref().is_some_and(|value| !value.is_empty());
    !set(&wayland_display) && !set(&wayland_socket) && set(&display)
}

/// Work area of the monitor under the pointer, or of the primary one.
pub(crate) fn work_area_under_cursor() -> Option<WorkArea> {
    let display = display()?;
    let pointer = display
        .connection
        .query_pointer(display.root)
        .ok()?
        .reply()
        .ok()?;
    let monitors = monitors(display)?;
    let monitor = monitor_at(&monitors, pointer.root_x.into(), pointer.root_y.into())?;
    Some(work_area(display, monitor))
}

pub(crate) fn primary_work_area() -> Option<WorkArea> {
    let display = display()?;
    let monitors = monitors(display)?;
    let monitor = primary_monitor(&monitors)?;
    Some(work_area(display, monitor))
}

fn monitors(display: &Display) -> Option<Vec<Monitor>> {
    let reply = display
        .connection
        .randr_get_monitors(display.root, true)
        .ok()?
        .reply()
        .ok()?;
    Some(
        reply
            .monitors
            .iter()
            .map(|monitor| Monitor {
                rect: Rect {
                    x: monitor.x.into(),
                    y: monitor.y.into(),
                    width: monitor.width.into(),
                    height: monitor.height.into(),
                },
                primary: monitor.primary,
                width_mm: monitor.width_in_millimeters,
                height_mm: monitor.height_in_millimeters,
            })
            .collect(),
    )
}

fn work_area(display: &Display, monitor: &Monitor) -> WorkArea {
    let usable =
        desktop_work_area(display).map_or(monitor.rect, |work| intersect(monitor.rect, work));
    WorkArea {
        x: usable.x,
        y: usable.y,
        width: usable.width,
        height: usable.height,
        scale: scale_factor(&resources(display), monitor),
    }
}

/// `_NET_WORKAREA` of the first desktop: the screen minus the panels.
fn desktop_work_area(display: &Display) -> Option<Rect> {
    let atom = atom(display, b"_NET_WORKAREA")?;
    let reply = display
        .connection
        .get_property(false, display.root, atom, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let mut values = reply.value32()?;
    Some(Rect {
        x: values.next()? as i32,
        y: values.next()? as i32,
        width: values.next()?,
        height: values.next()?,
    })
}

/// The X resources of the root window (`xrdb`), where desktops set `Xft.dpi`.
fn resources(display: &Display) -> String {
    display
        .connection
        .get_property(
            false,
            display.root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            u32::MAX / 4,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .unwrap_or_default()
}

/// Asks the window manager to activate `window` as a pager would (source 2), which window
/// managers honor even when they refuse requests from applications.
pub(crate) fn activate(window: u32) -> bool {
    display().is_some_and(|display| {
        send_to_root(display, window, b"_NET_ACTIVE_WINDOW", [2, 0, 0, 0, 0]).is_some()
    })
}

/// Keeps `window` out of the taskbar and the pager (`_NET_WM_STATE`, action 1 = add).
pub(crate) fn skip_taskbar(window: u32) {
    let Some(display) = display() else {
        return;
    };
    let (Some(taskbar), Some(pager)) = (
        atom(display, b"_NET_WM_STATE_SKIP_TASKBAR"),
        atom(display, b"_NET_WM_STATE_SKIP_PAGER"),
    ) else {
        return;
    };
    send_to_root(display, window, b"_NET_WM_STATE", [1, taskbar, pager, 1, 0]);
}

fn send_to_root(display: &Display, window: u32, kind: &[u8], data: [u32; 5]) -> Option<()> {
    let kind = atom(display, kind)?;
    let event = ClientMessageEvent::new(32, window, kind, data);
    display
        .connection
        .send_event(
            false,
            display.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .ok()?;
    display.connection.flush().ok()
}

/// Whether a compositor runs (it owns `_NET_WM_CM_S<screen>`), so transparent pixels show what
/// is behind the window instead of black.
pub(crate) fn compositor_running() -> bool {
    let Some(display) = display() else {
        return false;
    };
    let name = format!("_NET_WM_CM_S{}", display.screen);
    atom(display, name.as_bytes())
        .and_then(|selection| display.connection.get_selection_owner(selection).ok())
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.owner != x11rb::NONE)
}

fn atom(display: &Display, name: &[u8]) -> Option<u32> {
    Some(
        display
            .connection
            .intern_atom(false, name)
            .ok()?
            .reply()
            .ok()?
            .atom,
    )
}

/// The monitor that contains the point, else the primary one.
pub(crate) fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|monitor| monitor.rect.contains(x, y))
        .or_else(|| primary_monitor(monitors))
}

/// The primary monitor, else the first one.
pub(crate) fn primary_monitor(monitors: &[Monitor]) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or(monitors.first())
}

/// The part of `monitor` inside `work`; the whole monitor when they do not overlap.
pub(crate) fn intersect(monitor: Rect, work: Rect) -> Rect {
    let left = monitor.x.max(work.x);
    let top = monitor.y.max(work.y);
    let right = (i64::from(monitor.x) + i64::from(monitor.width))
        .min(i64::from(work.x) + i64::from(work.width));
    let bottom = (i64::from(monitor.y) + i64::from(monitor.height))
        .min(i64::from(work.y) + i64::from(work.height));
    if right <= i64::from(left) || bottom <= i64::from(top) {
        return monitor;
    }
    Rect {
        x: left,
        y: top,
        width: (right - i64::from(left)) as u32,
        height: (bottom - i64::from(top)) as u32,
    }
}

/// The scale winit gives windows on X11: `Xft.dpi / 96` when the resources set it, else the
/// monitor's pixel density in steps of 1/12, never below 1.
pub(crate) fn scale_factor(resources: &str, monitor: &Monitor) -> f32 {
    if let Some(dpi) = xft_dpi(resources) {
        return dpi / 96.0;
    }
    if monitor.width_mm == 0 || monitor.height_mm == 0 {
        return 1.0;
    }
    let pixels = f64::from(monitor.rect.width) * f64::from(monitor.rect.height);
    let millimeters = f64::from(monitor.width_mm) * f64::from(monitor.height_mm);
    let per_mm = (pixels / millimeters).sqrt();
    (((per_mm * (12.0 * 25.4 / 96.0)).round() / 12.0).max(1.0)) as f32
}

pub(crate) fn xft_dpi(resources: &str) -> Option<f32> {
    resources.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == "Xft.dpi")
            .then(|| value.trim().parse::<f32>().ok())
            .flatten()
            .filter(|dpi| *dpi > 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, width: u32, height: u32, primary: bool) -> Monitor {
        Monitor {
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            primary,
            width_mm: 0,
            height_mm: 0,
        }
    }

    #[test]
    fn x11_only_when_no_wayland_variable_is_set() {
        let some = |value: &str| Some(OsString::from(value));
        assert!(is_x11_session(None, None, some(":0")));
        assert!(is_x11_session(some(""), None, some(":0")));
        assert!(!is_x11_session(some("wayland-0"), None, some(":0")));
        assert!(!is_x11_session(None, some("5"), some(":0")));
        assert!(!is_x11_session(None, None, None));
        assert!(!is_x11_session(None, None, some("")));
    }

    #[test]
    fn the_monitor_under_the_pointer_wins() {
        let monitors = [
            monitor(0, 0, 1920, 1080, true),
            monitor(1920, 0, 2560, 1440, false),
        ];
        assert_eq!(monitor_at(&monitors, 2000, 100), Some(&monitors[1]));
        assert_eq!(monitor_at(&monitors, 1919, 1079), Some(&monitors[0]));
        assert_eq!(monitor_at(&monitors, -5, -5), Some(&monitors[0]));
        assert_eq!(monitor_at(&[], 0, 0), None);
    }

    #[test]
    fn without_a_primary_the_first_monitor_is_used() {
        let monitors = [
            monitor(0, 0, 800, 600, false),
            monitor(800, 0, 800, 600, false),
        ];
        assert_eq!(primary_monitor(&monitors), Some(&monitors[0]));
        let monitors = [
            monitor(0, 0, 800, 600, false),
            monitor(800, 0, 800, 600, true),
        ];
        assert_eq!(primary_monitor(&monitors), Some(&monitors[1]));
    }

    #[test]
    fn the_work_area_cuts_the_panels_out_of_the_monitor() {
        let screen = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let work = Rect {
            x: 0,
            y: 32,
            width: 3840,
            height: 1048,
        };
        assert_eq!(
            intersect(screen, work),
            Rect {
                x: 0,
                y: 32,
                width: 1920,
                height: 1048
            }
        );
        let elsewhere = Rect {
            x: 5000,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(intersect(screen, elsewhere), screen);
    }

    #[test]
    fn xft_dpi_sets_the_scale() {
        let plain = monitor(0, 0, 1920, 1080, true);
        assert_eq!(xft_dpi("Xft.antialias:\t1\nXft.dpi:\t144\n"), Some(144.0));
        assert_eq!(xft_dpi("Xft.dpi: abc"), None);
        assert_eq!(xft_dpi(""), None);
        assert_eq!(scale_factor("Xft.dpi:\t120", &plain), 1.25);
    }

    #[test]
    fn without_xft_dpi_the_density_sets_the_scale() {
        let mut laptop = monitor(0, 0, 1920, 1080, true);
        laptop.width_mm = 293;
        laptop.height_mm = 165;
        assert_eq!(scale_factor("", &laptop), 1.75);
        let mut office = monitor(0, 0, 1920, 1080, true);
        office.width_mm = 527;
        office.height_mm = 296;
        assert_eq!(scale_factor("", &office), 1.0);
        assert_eq!(scale_factor("", &monitor(0, 0, 1920, 1080, true)), 1.0);
    }
}
