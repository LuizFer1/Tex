#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const INFO_HEADER_LEN: usize = 40;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn rgba_len(width: u32, height: u32) -> Option<usize> {
    let len = u64::from(width) * u64::from(height) * 4;
    (width > 0 && height > 0 && len <= MAX_BYTES).then_some(len as usize)
}

struct Channel {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Channel {
        let shift = if mask == 0 { 0 } else { mask.trailing_zeros() };
        Channel {
            mask,
            shift,
            max: mask.checked_shr(shift).unwrap_or(0),
        }
    }

    fn read(&self, pixel: u32) -> u8 {
        if self.max == 0 {
            return 0;
        }
        let value = (pixel & self.mask) >> self.shift;
        (u64::from(value) * 255 / u64::from(self.max)) as u8
    }
}

enum Format {
    Indexed(Vec<[u8; 4]>),
    Masks([Channel; 4]),
    Bgr,
}

/// Reads a packed DIB (`CF_DIB`/`CF_DIBV5` bytes): BITMAPINFOHEADER or V4/V5 header,
/// 1, 4 or 8 bits with a color table, 16, 24 or 32 bits, BI_RGB or BI_BITFIELDS,
/// bottom-up or top-down. 32-bit images whose alpha is all zero are treated as opaque.
pub fn dib_to_rgba(dib: &[u8]) -> Option<Rgba> {
    let header_len = u32_at(dib, 0)? as usize;
    if header_len < INFO_HEADER_LEN || header_len > dib.len() {
        return None;
    }
    let width = i32::from_le_bytes(dib[4..8].try_into().ok()?);
    let height = i32::from_le_bytes(dib[8..12].try_into().ok()?);
    let bits = u16_at(dib, 14)?;
    let compression = u32_at(dib, 16)?;
    let palette = u32_at(dib, 32)? as usize;
    let top_down = height < 0;
    let width = u32::try_from(width).ok()?;
    let height = height.unsigned_abs();
    let len = rgba_len(width, height)?;

    let mut offset = header_len;
    let format = match (bits, compression) {
        (1 | 4 | 8, BI_RGB) => {
            let entries = if palette == 0 { 1 << bits } else { palette };
            let table_len = entries.checked_mul(4)?;
            let table = dib.get(offset..offset.checked_add(table_len)?)?;
            offset += table_len;
            Format::Indexed(
                table
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|color| [color[2], color[1], color[0], 255])
                    .collect(),
            )
        }
        (16, BI_RGB) => Format::Masks([
            Channel::new(0x7c00),
            Channel::new(0x03e0),
            Channel::new(0x001f),
            Channel::new(0),
        ]),
        (16 | 32, BI_BITFIELDS) => {
            if header_len == INFO_HEADER_LEN {
                offset += 12;
            }
            let alpha = if header_len >= 56 {
                u32_at(dib, 52)?
            } else {
                0
            };
            Format::Masks([
                Channel::new(u32_at(dib, 40)?),
                Channel::new(u32_at(dib, 44)?),
                Channel::new(u32_at(dib, 48)?),
                Channel::new(alpha),
            ])
        }
        (24 | 32, BI_RGB) => Format::Bgr,
        _ => return None,
    };
    if !matches!(format, Format::Indexed(_)) {
        offset = offset.checked_add(palette.checked_mul(4)?)?;
    }

    let bits = usize::from(bits);
    let stride = (width as usize * bits).div_ceil(32) * 4;
    let row_len = (width as usize * bits).div_ceil(8);
    let data = dib.get(offset..offset.checked_add(stride.checked_mul(height as usize)?)?)?;

    let mut pixels = Vec::with_capacity(len);
    for y in 0..height as usize {
        let row = if top_down { y } else { height as usize - 1 - y };
        let row = &data[row * stride..row * stride + row_len];
        match &format {
            Format::Indexed(table) => {
                for x in 0..width as usize {
                    let bit = x * bits;
                    let index = (row[bit / 8] >> (8 - bits - bit % 8)) & ((1u16 << bits) - 1) as u8;
                    pixels.extend(
                        table
                            .get(usize::from(index))
                            .copied()
                            .unwrap_or([0, 0, 0, 255]),
                    );
                }
            }
            Format::Masks([r, g, b, a]) => {
                for px in row.chunks_exact(bits / 8) {
                    let value = match *px {
                        [low, high] => u32::from(u16::from_le_bytes([low, high])),
                        _ => u32::from_le_bytes(px.try_into().ok()?),
                    };
                    let alpha = if a.max == 0 { 255 } else { a.read(value) };
                    pixels.extend([r.read(value), g.read(value), b.read(value), alpha]);
                }
            }
            Format::Bgr => {
                for px in row.chunks_exact(bits / 8) {
                    let alpha = if bits == 32 { px[3] } else { 255 };
                    pixels.extend([px[2], px[1], px[0], alpha]);
                }
            }
        }
    }
    if bits == 32
        && matches!(format, Format::Bgr)
        && pixels.as_chunks::<4>().0.iter().all(|px| px[3] == 0)
    {
        pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|px| px[3] = 255);
    }
    Some(Rgba {
        width,
        height,
        pixels,
    })
}

/// 40-byte BITMAPINFOHEADER, 32 bits, BI_RGB, bottom-up: the DIB every app reads.
pub fn rgba_to_dib(image: &Rgba) -> Vec<u8> {
    let row_len = image.width as usize * 4;
    let mut dib = Vec::with_capacity(INFO_HEADER_LEN + image.pixels.len());
    dib.extend((INFO_HEADER_LEN as u32).to_le_bytes());
    dib.extend((image.width as i32).to_le_bytes());
    dib.extend((image.height as i32).to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(BI_RGB.to_le_bytes());
    dib.extend((image.pixels.len() as u32).to_le_bytes());
    dib.resize(INFO_HEADER_LEN, 0);
    for row in image.pixels.chunks_exact(row_len).rev() {
        for px in row.as_chunks::<4>().0 {
            dib.extend([px[2], px[1], px[0], px[3]]);
        }
    }
    dib
}

pub fn encode_png(image: &Rgba) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .expect("PNG header for a non-empty image");
    writer
        .write_image_data(&image.pixels)
        .expect("pixels match the image size");
    writer.finish().expect("PNG written to memory");
    out
}

pub fn decode_png(png: &[u8]) -> Option<Rgba> {
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(png),
        png::Limits {
            bytes: MAX_BYTES as usize,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    rgba_len(info.width, info.height)?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let len = rgba_len(frame.width, frame.height)?;
    let mut pixels = Vec::with_capacity(len);
    for row in buffer
        .chunks_exact(frame.line_size)
        .take(frame.height as usize)
    {
        let row = &row[..frame.width as usize * frame.color_type.samples()];
        match frame.color_type {
            png::ColorType::Rgba => pixels.extend_from_slice(row),
            png::ColorType::Rgb => row
                .as_chunks::<3>()
                .0
                .iter()
                .for_each(|px| pixels.extend([px[0], px[1], px[2], 255])),
            png::ColorType::GrayscaleAlpha => row
                .as_chunks::<2>()
                .0
                .iter()
                .for_each(|px| pixels.extend([px[0], px[0], px[0], px[1]])),
            png::ColorType::Grayscale => row.iter().for_each(|&v| pixels.extend([v, v, v, 255])),
            png::ColorType::Indexed => return None,
        }
    }
    Some(Rgba {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}

/// Box-filter downscale so the longer side is at most `max` (never upscales).
pub fn thumbnail(image: &Rgba, max: u32) -> Rgba {
    let (width, height) = (image.width, image.height);
    let max = max.max(1);
    if width <= max && height <= max {
        return image.clone();
    }
    let scaled = |side: u32, longer: u32| {
        ((u64::from(side) * u64::from(max) + u64::from(longer) / 2) / u64::from(longer)).max(1)
            as u32
    };
    let longer = width.max(height);
    let (new_width, new_height) = (scaled(width, longer), scaled(height, longer));
    let span = |i: u32, new: u32, old: u32| {
        let start = (u64::from(i) * u64::from(old) / u64::from(new)) as usize;
        let end = ((u64::from(i + 1) * u64::from(old) / u64::from(new)) as usize).max(start + 1);
        start..end
    };

    let mut pixels = Vec::with_capacity(new_width as usize * new_height as usize * 4);
    for y in 0..new_height {
        let rows = span(y, new_height, height);
        for x in 0..new_width {
            let columns = span(x, new_width, width);
            let mut sums = [0u64; 4];
            let mut count = 0u64;
            for row in rows.clone() {
                let start = (row * width as usize + columns.start) * 4;
                let end = (row * width as usize + columns.end) * 4;
                for px in image.pixels[start..end].as_chunks::<4>().0 {
                    let alpha = u64::from(px[3]);
                    for (sum, &value) in sums.iter_mut().zip(&px[..3]) {
                        *sum += u64::from(value) * alpha;
                    }
                    sums[3] += alpha;
                    count += 1;
                }
            }
            let color = |sum: u64| {
                (sum + sums[3] / 2)
                    .checked_div(sums[3])
                    .map_or(0, |value| value as u8)
            };
            pixels.extend([
                color(sums[0]),
                color(sums[1]),
                color(sums[2]),
                ((sums[3] + count / 2) / count) as u8,
            ]);
        }
    }
    Rgba {
        width: new_width,
        height: new_height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(size: u32, width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(size.to_le_bytes());
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend(compression.to_le_bytes());
        out.resize(size as usize, 0);
        out
    }

    fn masks(dib: &mut [u8], at: usize, masks: &[u32]) {
        for (i, mask) in masks.iter().enumerate() {
            dib[at + i * 4..at + i * 4 + 4].copy_from_slice(&mask.to_le_bytes());
        }
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn pixels(rows: &[[[u8; 4]; 2]]) -> Vec<u8> {
        rows.iter().flatten().flatten().copied().collect()
    }

    #[test]
    fn reads_24_bit_bottom_up() {
        let mut dib = header(40, 2, 2, 24, 0);
        dib.extend([255, 0, 0, 255, 255, 255, 0, 0]);
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        let image = dib_to_rgba(&dib).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.pixels, pixels(&[[RED, GREEN], [BLUE, WHITE]]));
    }

    #[test]
    fn reads_32_bit_top_down() {
        let mut dib = header(40, 2, -2, 32, 0);
        dib.extend([0, 0, 255, 128, 0, 255, 0, 255]);
        dib.extend([255, 0, 0, 255, 255, 255, 255, 0]);
        let image = dib_to_rgba(&dib).unwrap();
        assert_eq!(
            image.pixels,
            pixels(&[[[255, 0, 0, 128], GREEN], [BLUE, [255, 255, 255, 0]]])
        );
    }

    #[test]
    fn zero_alpha_everywhere_is_opaque() {
        let mut dib = header(40, 2, -2, 32, 0);
        dib.extend([0, 0, 255, 0, 0, 255, 0, 0]);
        dib.extend([255, 0, 0, 0, 255, 255, 255, 0]);
        let image = dib_to_rgba(&dib).unwrap();
        assert_eq!(image.pixels, pixels(&[[RED, GREEN], [BLUE, WHITE]]));
    }

    #[test]
    fn reads_bitfields_after_a_40_byte_header() {
        let mut dib = header(40, 2, -1, 32, 3);
        dib.extend([0; 12]);
        masks(&mut dib, 40, &[0x0000_00ff, 0x0000_ff00, 0x00ff_0000]);
        dib.extend([255, 0, 0, 0, 0, 0, 255, 0]);
        let image = dib_to_rgba(&dib).unwrap();
        assert_eq!(image.pixels, [RED, BLUE].concat());
    }

    #[test]
    fn reads_bitfields_in_a_v5_header() {
        let mut dib = header(124, 1, 1, 32, 3);
        masks(
            &mut dib,
            40,
            &[0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000],
        );
        dib.extend([10, 20, 30, 40]);
        let image = dib_to_rgba(&dib).unwrap();
        assert_eq!(image.pixels, [30, 20, 10, 40]);
    }

    #[test]
    fn reads_1_bit_with_its_color_table() {
        let mut dib = header(40, 9, -1, 1, 0);
        dib.extend([0, 0, 0, 0, 255, 255, 255, 0]);
        dib.extend([0b1010_0000, 0b1000_0000, 0, 0]);
        let image = dib_to_rgba(&dib).unwrap();
        let black = [0, 0, 0, 255];
        assert_eq!(
            image.pixels,
            [
                WHITE, black, WHITE, black, black, black, black, black, WHITE
            ]
            .concat()
        );
    }

    #[test]
    fn reads_4_and_8_bit_with_a_short_color_table() {
        let mut dib = header(40, 2, -1, 4, 0);
        dib[32..36].copy_from_slice(&2u32.to_le_bytes());
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        dib.extend([0x01, 0, 0, 0]);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, [RED, BLUE].concat());

        let mut dib = header(40, 3, -1, 8, 0);
        dib[32..36].copy_from_slice(&2u32.to_le_bytes());
        dib.extend([0, 255, 0, 0, 255, 255, 255, 0]);
        dib.extend([1, 0, 7, 0]);
        assert_eq!(
            dib_to_rgba(&dib).unwrap().pixels,
            [WHITE, GREEN, [0, 0, 0, 255]].concat()
        );
    }

    #[test]
    fn reads_16_bit_555_and_565() {
        let mut dib = header(40, 2, -1, 16, 0);
        dib.extend([0x00, 0x7c, 0x1f, 0x00]);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, [RED, BLUE].concat());

        let mut dib = header(40, 2, -1, 16, 3);
        dib.extend([0; 12]);
        masks(&mut dib, 40, &[0xf800, 0x07e0, 0x001f]);
        dib.extend([0xe0, 0x07, 0xff, 0xff]);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, [GREEN, WHITE].concat());
    }

    #[test]
    fn rejects_unsupported_and_truncated() {
        let mut dib = header(40, 2, 2, 8, 1);
        dib.extend([0; 64]);
        assert_eq!(dib_to_rgba(&dib), None);
        let mut dib = header(40, 2, 2, 24, 0);
        dib.extend([0; 15]);
        assert_eq!(dib_to_rgba(&dib), None);
        assert_eq!(dib_to_rgba(&header(40, 2, 2, 24, 0)[..30]), None);
        let mut dib = header(40, 0, 2, 24, 0);
        dib.extend([0; 16]);
        assert_eq!(dib_to_rgba(&dib), None);
        assert_eq!(dib_to_rgba(&header(40, 100_000, 100_000, 32, 0)), None);
    }

    fn sample(width: u32, height: u32) -> Rgba {
        let pixels = (0..width * height)
            .flat_map(|i| [(i * 7) as u8, (i * 13) as u8, (i * 29) as u8, 200])
            .collect();
        Rgba {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn dib_round_trip() {
        let image = sample(3, 5);
        let dib = rgba_to_dib(&image);
        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 40);
        assert_eq!(dib_to_rgba(&dib), Some(image));
    }

    #[test]
    fn png_round_trip() {
        let image = sample(7, 4);
        let png = encode_png(&image);
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(decode_png(&png), Some(image));
        assert_eq!(decode_png(b"not a png"), None);
    }

    #[test]
    fn thumbnail_keeps_the_aspect_and_never_upscales() {
        let wide = thumbnail(&sample(300, 100), 96);
        assert_eq!((wide.width, wide.height), (96, 32));
        assert_eq!(wide.pixels.len(), 96 * 32 * 4);
        let tall = thumbnail(&sample(10, 200), 96);
        assert_eq!((tall.width, tall.height), (5, 96));
        let small = sample(50, 50);
        assert_eq!(thumbnail(&small, 96), small);
    }

    #[test]
    fn thumbnail_averages_boxes() {
        let image = Rgba {
            width: 2,
            height: 2,
            pixels: pixels(&[[RED, RED], [BLUE, [0, 0, 0, 0]]]),
        };
        let small = thumbnail(&image, 1);
        assert_eq!((small.width, small.height), (1, 1));
        assert_eq!(small.pixels, [170, 0, 85, 191]);
    }
}
