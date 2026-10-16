#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{SingleInstance, work_area_under_cursor};

#[cfg(not(windows))]
compile_error!("tex-platform only supports Windows for now");

/// Usable area of a monitor (screen minus taskbar) in physical pixels, with its DPI scale (1.0 = 96 DPI).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}
