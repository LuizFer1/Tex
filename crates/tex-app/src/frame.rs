use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::window::ResizeDirection;

const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// Wires the title bar and edges of a frameless window: its `drag`, `resize`, `minimize`,
/// `toggle-maximize` and `close-window` callbacks and its `window-maximized` property. Each window
/// keeps its own double-click time.
macro_rules! wire_frame {
    ($window:expr) => {{
        use slint::ComponentHandle as _;
        let window = $window;
        window.on_drag({
            let weak = window.as_weak();
            let pressed_at = ::std::cell::Cell::new(None);
            move || {
                if let Some(window) = weak.upgrade()
                    && $crate::frame::drag(window.window(), &pressed_at)
                {
                    window.set_window_maximized($crate::frame::toggle_maximized(window.window()));
                }
            }
        });
        window.on_resize({
            let weak = window.as_weak();
            move |direction| {
                if let Some(window) = weak.upgrade() {
                    $crate::frame::resize(window.window(), direction);
                }
            }
        });
        window.on_minimize({
            let weak = window.as_weak();
            move || {
                if let Some(window) = weak.upgrade() {
                    window.window().set_minimized(true);
                }
            }
        });
        window.on_toggle_maximize({
            let weak = window.as_weak();
            move || {
                if let Some(window) = weak.upgrade() {
                    window.set_window_maximized($crate::frame::toggle_maximized(window.window()));
                }
            }
        });
        window.on_close_window({
            let weak = window.as_weak();
            move || {
                if let Some(window) = weak.upgrade() {
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::CloseRequested);
                }
            }
        });
    }};
}

pub(crate) use wire_frame;

/// Whether a press at `now` follows the previous one closely enough to be a double click.
pub fn is_double_click(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_some_and(|previous| now.saturating_duration_since(previous) <= DOUBLE_CLICK)
}

/// Starts moving the window, or returns true on a double click, which the caller turns into maximize.
pub fn drag(window: &slint::Window, pressed_at: &Cell<Option<Instant>>) -> bool {
    let now = Instant::now();
    if is_double_click(pressed_at.take(), now) {
        return true;
    }
    pressed_at.set(Some(now));
    window.with_winit_window(|window| {
        let _ = window.drag_window();
    });
    false
}

/// Maximizes or restores the window and returns whether it is now maximized.
pub fn toggle_maximized(window: &slint::Window) -> bool {
    let maximized = !window.is_maximized();
    window.set_maximized(maximized);
    maximized
}

pub fn resize(window: &slint::Window, direction: i32) {
    window.with_winit_window(|window| {
        let _ = window.drag_resize_window(resize_direction(direction));
    });
}

/// Order matches the `direction` each window gives its `ResizeEdge`s.
fn resize_direction(direction: i32) -> ResizeDirection {
    match direction {
        0 => ResizeDirection::North,
        1 => ResizeDirection::South,
        2 => ResizeDirection::East,
        3 => ResizeDirection::West,
        4 => ResizeDirection::NorthEast,
        5 => ResizeDirection::NorthWest,
        6 => ResizeDirection::SouthEast,
        _ => ResizeDirection::SouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_press_within_half_a_second_is_a_double_click() {
        let start = Instant::now();
        assert!(!is_double_click(None, start));
        assert!(is_double_click(
            Some(start),
            start + Duration::from_millis(400)
        ));
        assert!(!is_double_click(
            Some(start),
            start + Duration::from_millis(600)
        ));
    }
}
