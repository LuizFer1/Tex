use tex_platform::WorkArea;

pub const POPUP_WIDTH: f32 = 640.0;
pub const HISTORY_WIDTH: f32 = 820.0;

/// Top-left corner in physical pixels: centered horizontally, top edge at a quarter of the work area height.
pub fn popup_origin(area: &WorkArea, logical_width: f32) -> (i32, i32) {
    let width = (logical_width * area.scale).round() as u32;
    let x = area.x + (area.width.saturating_sub(width) / 2) as i32;
    let y = area.y + (area.height / 4) as i32;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, width: u32, height: u32, scale: f32) -> WorkArea {
        WorkArea {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn centers_on_primary_monitor() {
        assert_eq!(
            popup_origin(&area(0, 0, 1920, 1040, 1.0), 640.0),
            (640, 260)
        );
    }

    #[test]
    fn scales_width_by_monitor_dpi() {
        assert_eq!(
            popup_origin(&area(0, 0, 2560, 1400, 1.5), 640.0),
            (800, 350)
        );
    }

    #[test]
    fn offsets_by_monitor_position() {
        assert_eq!(
            popup_origin(&area(-1920, 40, 1920, 1040, 1.0), 640.0),
            (-1280, 300)
        );
    }

    #[test]
    fn pins_to_left_edge_when_popup_is_wider_than_monitor() {
        assert_eq!(
            popup_origin(&area(100, 0, 500, 800, 1.0), 640.0),
            (100, 200)
        );
    }
}
