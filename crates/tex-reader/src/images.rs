use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader, Limits};

pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 64 * 1_000_000;
pub const MAX_FRAMES: usize = 300;
pub const MAX_RGBA_BYTES: u64 = 128 * 1024 * 1024;
const MIN_DELAY_MS: u32 = 20;
const DEFAULT_DELAY_MS: u32 = 100;

/// Why an image is not shown; the codes are the ones `ui/reader.slint` turns into sentences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Unsupported = 1,
    Unreadable = 2,
    Network = 3,
    TooLarge = 4,
    Corrupt = 5,
    Insecure = 6,
    RemoteOff = 7,
    Budget = 8,
}

impl Reason {
    pub fn code(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Local(PathBuf),
    Remote(String),
    Unsupported(Reason),
}

/// Where the image `src` of a Markdown file in `md_dir` lives.
pub fn resolve(src: &str, md_dir: &Path) -> Source {
    let src = src.trim();
    if src.is_empty() || has_prefix(src, "data:") || is_svg(src) {
        return Source::Unsupported(Reason::Unsupported);
    }
    if has_prefix(src, "https://") {
        return Source::Remote(src.to_owned());
    }
    if has_prefix(src, "http://") {
        return Source::Unsupported(Reason::Insecure);
    }
    if has_prefix(src, "file://") {
        return file_uri(&src[7..]);
    }
    let decoded = percent_decode(src);
    let path = PathBuf::from(&decoded);
    if path.is_absolute() || has_drive(&decoded) {
        Source::Local(path)
    } else {
        Source::Local(md_dir.join(decoded))
    }
}

fn has_prefix(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_svg(src: &str) -> bool {
    let end = src.find(['?', '#']).unwrap_or(src.len());
    src[..end].to_ascii_lowercase().ends_with(".svg")
}

fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn file_uri(rest: &str) -> Source {
    let decoded = percent_decode(rest);
    if let Some(path) = decoded.strip_prefix('/') {
        if has_drive(path) {
            return Source::Local(PathBuf::from(path));
        }
        return Source::Local(PathBuf::from(decoded));
    }
    let (host, path) = decoded.split_once('/').unwrap_or((decoded.as_str(), ""));
    if host.is_empty() {
        return Source::Unsupported(Reason::Unsupported);
    }
    Source::Local(PathBuf::from(format!(
        "\\\\{host}\\{}",
        path.replace('/', "\\")
    )))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2]))
        {
            out.push(high * 16 + low);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub frames: Vec<Frame>,
    pub width: u32,
    pub height: u32,
}

/// Fails with `TooLarge` when `frames` frames of `width` x `height` pass a limit, `Corrupt` on a
/// zero side.
fn check_size(width: u32, height: u32, frames: usize) -> Result<(), Reason> {
    if width == 0 || height == 0 {
        return Err(Reason::Corrupt);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS
        || frames > MAX_FRAMES
        || pixels * 4 * frames.max(1) as u64 > MAX_RGBA_BYTES
    {
        return Err(Reason::TooLarge);
    }
    Ok(())
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_RGBA_BYTES + 16 * 1024 * 1024);
    limits
}

fn map_error(error: image::ImageError) -> Reason {
    match error {
        image::ImageError::Limits(_) => Reason::TooLarge,
        image::ImageError::Unsupported(_) => Reason::Unsupported,
        _ => Reason::Corrupt,
    }
}

fn delay_ms(numer: u32, denom: u32) -> u32 {
    let ms = numer / denom.max(1);
    if ms < MIN_DELAY_MS {
        DEFAULT_DELAY_MS
    } else {
        ms
    }
}

/// Decodes PNG, JPEG, GIF, WebP or BMP bytes, chosen by content; a GIF keeps every frame.
pub fn decode(bytes: &[u8]) -> Result<Decoded, Reason> {
    let format = match image::guess_format(bytes) {
        Ok(
            format @ (ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp),
        ) => format,
        _ => return Err(Reason::Unsupported),
    };
    if format == ImageFormat::Gif {
        return decode_gif(bytes);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let (width, height) = reader.into_dimensions().map_err(map_error)?;
    check_size(width, height, 1)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let rgba = reader.decode().map_err(map_error)?.into_rgba8();
    Ok(Decoded {
        width,
        height,
        frames: vec![Frame {
            rgba: rgba.into_raw(),
            delay_ms: 0,
        }],
    })
}

fn decode_gif(bytes: &[u8]) -> Result<Decoded, Reason> {
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(map_error)?;
    decoder.set_limits(limits()).map_err(map_error)?;
    let (width, height) = decoder.dimensions();
    check_size(width, height, 1)?;
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.map_err(map_error)?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay = delay_ms(numer, denom);
        check_size(width, height, frames.len() + 1)?;
        frames.push(Frame {
            rgba: frame.into_buffer().into_raw(),
            delay_ms: delay,
        });
    }
    if frames.is_empty() {
        return Err(Reason::Corrupt);
    }
    Ok(Decoded {
        width,
        height,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dir() -> &'static Path {
        Path::new("docs")
    }

    #[test]
    fn relative_paths_join_the_markdown_folder() {
        assert_eq!(
            resolve("img/logo.png", dir()),
            Source::Local(Path::new("docs").join("img/logo.png"))
        );
        assert_eq!(
            resolve("my%20pic.png", dir()),
            Source::Local(Path::new("docs").join("my pic.png"))
        );
    }

    #[test]
    fn remote_sources_need_https() {
        assert_eq!(
            resolve("https://example.com/a.png?x=1", dir()),
            Source::Remote("https://example.com/a.png?x=1".into())
        );
        assert_eq!(
            resolve("HTTPS://example.com/a.png", dir()),
            Source::Remote("HTTPS://example.com/a.png".into())
        );
        assert_eq!(
            resolve("http://example.com/a.png", dir()),
            Source::Unsupported(Reason::Insecure)
        );
    }

    #[test]
    fn data_uris_svg_and_empty_sources_are_unsupported() {
        for src in [
            "data:image/png;base64,AAAA",
            "logo.svg",
            "logo.SVG",
            "https://x.org/a.svg?v=2",
            "",
            "  ",
        ] {
            assert_eq!(
                resolve(src, dir()),
                Source::Unsupported(Reason::Unsupported),
                "{src}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn file_uris_and_drive_paths() {
        assert_eq!(
            resolve("file:///C:/a%20b/x.png", dir()),
            Source::Local(std::path::PathBuf::from("C:/a b/x.png"))
        );
        assert_eq!(
            resolve("file://server/share/x.png", dir()),
            Source::Local(std::path::PathBuf::from("\\\\server\\share\\x.png"))
        );
        assert_eq!(
            resolve("C:\\fotos\\x.png", dir()),
            Source::Local(std::path::PathBuf::from("C:\\fotos\\x.png"))
        );
    }

    use image::codecs::gif::GifEncoder;
    use image::{Delay, DynamicImage, Frame as GifFrame, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(color))
    }

    fn encode(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut out, format)
            .unwrap();
        out.into_inner()
    }

    fn gif(delays: &[u32], width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for (index, delay) in delays.iter().enumerate() {
                let frame = GifFrame::from_parts(
                    solid(width, height, [index as u8 * 40, 0, 0, 255]),
                    0,
                    0,
                    Delay::from_numer_denom_ms(*delay, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    #[test]
    fn decodes_each_static_format() {
        let image = solid(3, 2, [10, 20, 30, 255]);
        for format in [ImageFormat::Png, ImageFormat::Bmp, ImageFormat::WebP] {
            let decoded = decode(&encode(&image, format)).unwrap();
            assert_eq!((decoded.width, decoded.height), (3, 2), "{format:?}");
            assert_eq!(decoded.frames.len(), 1);
            assert_eq!(decoded.frames[0].rgba.len(), 3 * 2 * 4);
        }
        let rgb = DynamicImage::ImageRgba8(image).to_rgb8();
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(rgb)
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(decode(&jpeg.into_inner()).unwrap().width, 3);
    }

    #[test]
    fn gif_frames_keep_their_delays_and_short_ones_become_100_ms() {
        let decoded = decode(&gif(&[0, 10, 40, 500], 2, 2)).unwrap();
        let delays: Vec<u32> = decoded.frames.iter().map(|frame| frame.delay_ms).collect();
        assert_eq!(delays, [100, 100, 40, 500]);
        assert!(decoded.frames.iter().all(|frame| frame.rgba.len() == 16));
    }

    #[test]
    fn rejects_what_is_not_an_image_or_is_not_listed() {
        assert_eq!(decode(b"hello"), Err(Reason::Unsupported));
        assert_eq!(decode(b""), Err(Reason::Unsupported));
        let mut png = encode(&solid(4, 4, [0, 0, 0, 255]), ImageFormat::Png);
        png.truncate(png.len() / 2);
        assert_eq!(decode(&png), Err(Reason::Corrupt));
    }

    #[test]
    fn rejects_images_over_the_limits() {
        assert_eq!(check_size(9000, 9000, 1), Err(Reason::TooLarge));
        assert_eq!(check_size(4000, 4000, 9), Err(Reason::TooLarge));
        assert_eq!(check_size(100, 100, MAX_FRAMES + 1), Err(Reason::TooLarge));
        assert_eq!(check_size(4000, 4000, 1), Ok(()));
        assert_eq!(check_size(0, 10, 1), Err(Reason::Corrupt));
    }
}
