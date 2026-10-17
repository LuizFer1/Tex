use tex_platform::WorkArea;

pub const POPUP_WIDTH: f32 = 700.0;
pub const HISTORY_WIDTH: f32 = 640.0;

/// Rounds a logical length to physical pixels at the given scale.
pub fn physical(logical: f32, scale: f32) -> i32 {
    (logical * scale).round() as i32
}

/// Top-left corner in physical pixels: centered horizontally, top edge at 15% of the work area height.
pub fn popup_origin(area: &WorkArea, logical_width: f32) -> (i32, i32) {
    let width = (logical_width * area.scale).round() as u32;
    let x = area.x + (area.width.saturating_sub(width) / 2) as i32;
    let y = area.y + (area.height * 15 / 100) as i32;
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
            (640, 156)
        );
    }

    #[test]
    fn scales_width_by_monitor_dpi() {
        assert_eq!(
            popup_origin(&area(0, 0, 2560, 1400, 1.5), 640.0),
            (800, 210)
        );
    }

    #[test]
    fn offsets_by_monitor_position() {
        assert_eq!(
            popup_origin(&area(-1920, 40, 1920, 1040, 1.0), 640.0),
            (-1280, 196)
        );
    }

    #[test]
    fn pins_to_left_edge_when_popup_is_wider_than_monitor() {
        assert_eq!(
            popup_origin(&area(100, 0, 500, 800, 1.0), 640.0),
            (100, 120)
        );
    }

    #[test]
    fn physical_rounds_logical_lengths() {
        assert_eq!(physical(6.0, 1.0), 6);
        assert_eq!(physical(6.0, 1.25), 8);
        assert_eq!(physical(26.0, 1.5), 39);
    }
}
