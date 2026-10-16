use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

const SIZE: u32 = 32;
const ACCENT: Rgba8Pixel = Rgba8Pixel::new(0x25, 0x63, 0xeb, 0xff);
const WHITE: Rgba8Pixel = Rgba8Pixel::new(0xff, 0xff, 0xff, 0xff);
const CLEAR: Rgba8Pixel = Rgba8Pixel::new(0, 0, 0, 0);

/// Draws the placeholder tray icon: a white "T" on a rounded blue square.
pub fn tray_icon() -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(SIZE, SIZE);
    for (i, pixel) in buffer.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (i as u32 % SIZE, i as u32 / SIZE);
        *pixel = if !inside_rounded_square(x, y) {
            CLEAR
        } else if is_letter_t(x, y) {
            WHITE
        } else {
            ACCENT
        };
    }
    Image::from_rgba8(buffer)
}

fn inside_rounded_square(x: u32, y: u32) -> bool {
    const RADIUS: i32 = 7;
    let max = SIZE as i32 - 1;
    let cx = (x as i32).clamp(RADIUS, max - RADIUS);
    let cy = (y as i32).clamp(RADIUS, max - RADIUS);
    let (dx, dy) = (x as i32 - cx, y as i32 - cy);
    dx * dx + dy * dy <= RADIUS * RADIUS
}

fn is_letter_t(x: u32, y: u32) -> bool {
    let bar = (7..25).contains(&x) && (7..11).contains(&y);
    let stem = (14..18).contains(&x) && (7..26).contains(&y);
    bar || stem
}
