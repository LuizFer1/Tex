use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::IconPixels;

const ICON_SIZE: u32 = 48;
const THEMES: [&str; 3] = ["hicolor", "Adwaita", "breeze"];
const SIZES: [&str; 9] = [
    "48x48", "64x64", "scalable", "96x96", "128x128", "256x256", "32x32", "24x24", "16x16",
];
const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;

/// The unlocalized value of `key` in the `[Desktop Entry]` group of a `.desktop` file, with the
/// escapes `\s`, `\n`, `\t`, `\r` and `\\` undone.
pub(crate) fn entry_value(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(unescape(value.trim()));
        }
    }
    None
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The program and arguments of an `Exec` value: quoted arguments keep their spaces, field
/// codes (`%f`, `%U`, `%i`…) are dropped and `%%` is a percent sign.
pub(crate) fn exec_args(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                    started = true;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args.retain(|arg| !arg.is_empty());
    args
}

/// Where to look for the icon named `name`: each theme in every base folder (`<base>/icons`),
/// sizes near 48 pixels first, then `/usr/share/pixmaps`.
pub(crate) fn icon_candidates(name: &str, bases: &[PathBuf]) -> Vec<PathBuf> {
    let name = name
        .strip_suffix(".png")
        .or_else(|| name.strip_suffix(".svg"))
        .unwrap_or(name);
    let mut candidates = Vec::new();
    for theme in THEMES {
        for base in bases {
            let theme_dir = base.join("icons").join(theme);
            for size in SIZES {
                let extension = if size == "scalable" { "svg" } else { "png" };
                candidates.push(
                    theme_dir
                        .join(size)
                        .join("apps")
                        .join(format!("{name}.{extension}")),
                );
            }
            candidates.push(
                theme_dir
                    .join("apps")
                    .join("48")
                    .join(format!("{name}.svg")),
            );
        }
    }
    for extension in ["png", "svg"] {
        candidates.push(PathBuf::from(format!(
            "/usr/share/pixmaps/{name}.{extension}"
        )));
    }
    candidates
}

/// The data folders whose `icons` hold app icons: the user's, the flatpak exports and
/// `XDG_DATA_DIRS` (by default `/usr/local/share` and `/usr/share`), without repeats.
pub(crate) fn icon_bases(home: &Path, data_dirs: Option<&str>) -> Vec<PathBuf> {
    let mut bases = vec![
        home.join(".local/share"),
        home.join(".local/share/flatpak/exports/share"),
    ];
    let system = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    bases.extend(
        system
            .split(':')
            .filter(|dir| dir.starts_with('/'))
            .map(PathBuf::from),
    );
    bases.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    let mut unique = Vec::new();
    for base in bases {
        if !unique.contains(&base) {
            unique.push(base);
        }
    }
    unique
}

/// Box-filter downscale by a whole factor so neither side passes `max`. Colors are averaged
/// weighted by alpha.
pub(crate) fn shrink(icon: IconPixels, max: u32) -> IconPixels {
    let longer = icon.width.max(icon.height);
    if longer <= max {
        return icon;
    }
    let factor = longer.div_ceil(max.max(1));
    let width = (icon.width / factor).max(1);
    let height = (icon.height / factor).max(1);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let mut sums = [0u64; 4];
            let mut count = 0u64;
            for row in y * factor..((y + 1) * factor).min(icon.height) {
                for column in x * factor..((x + 1) * factor).min(icon.width) {
                    let at = (row as usize * icon.width as usize + column as usize) * 4;
                    let pixel = &icon.rgba[at..at + 4];
                    let alpha = u64::from(pixel[3]);
                    for (sum, &value) in sums.iter_mut().zip(&pixel[..3]) {
                        *sum += u64::from(value) * alpha;
                    }
                    sums[3] += alpha;
                    count += 1;
                }
            }
            let color = |sum: u64| sum.checked_div(sums[3]).map_or(0, |value| value as u8);
            rgba.extend([
                color(sums[0]),
                color(sums[1]),
                color(sums[2]),
                (sums[3] / count.max(1)) as u8,
            ]);
        }
    }
    IconPixels {
        width,
        height,
        rgba,
    }
}

/// The icon of the app whose `.desktop` file is at `path`, from its `Icon=` key and the icon
/// themes, at most 48 pixels wide.
pub fn app_icon(path: &str) -> Option<IconPixels> {
    let text = fs::read_to_string(path).ok()?;
    let name = entry_value(&text, "Icon")?;
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return load_icon(Path::new(&name));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok();
    icon_candidates(&name, &icon_bases(&home, data_dirs.as_deref()))
        .into_iter()
        .filter(|candidate| candidate.is_file())
        .find_map(|candidate| load_icon(&candidate))
}

fn load_icon(path: &Path) -> Option<IconPixels> {
    if fs::metadata(path).ok()?.len() > MAX_ICON_BYTES {
        return None;
    }
    let icon = if path.extension().is_some_and(|extension| extension == "svg") {
        render_svg(&fs::read(path).ok()?)?
    } else {
        decode_png(path)?
    };
    Some(shrink(icon, ICON_SIZE))
}

fn render_svg(data: &[u8]) -> Option<IconPixels> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = ICON_SIZE as f32 / size.width().max(size.height());
    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(IconPixels {
        width: ICON_SIZE,
        height: ICON_SIZE,
        rgba: pixmap.take_demultiplied(),
    })
}

fn decode_png(path: &Path) -> Option<IconPixels> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path).ok()?));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let mut rgba = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
    for row in buffer
        .chunks_exact(frame.line_size)
        .take(frame.height as usize)
    {
        let row = &row[..frame.width as usize * frame.color_type.samples()];
        match frame.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(row),
            png::ColorType::Rgb => row
                .as_chunks::<3>()
                .0
                .iter()
                .for_each(|px| rgba.extend([px[0], px[1], px[2], 255])),
            png::ColorType::GrayscaleAlpha => row
                .as_chunks::<2>()
                .0
                .iter()
                .for_each(|px| rgba.extend([px[0], px[0], px[0], px[1]])),
            png::ColorType::Grayscale => row.iter().for_each(|&v| rgba.extend([v, v, v, 255])),
            png::ColorType::Indexed => return None,
        }
    }
    Some(IconPixels {
        width: frame.width,
        height: frame.height,
        rgba,
    })
}

/// Starts the app of the `.desktop` file at `path`: through `gio launch` when it exists,
/// otherwise by running its `Exec` line in its `Path` folder.
pub(crate) fn launch(path: &str) -> io::Result<()> {
    let mut gio = Command::new("gio");
    gio.args(["launch", path]);
    if super::spawn(gio).is_ok() {
        return Ok(());
    }
    let text = fs::read_to_string(path)?;
    let exec = entry_value(&text, "Exec")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Exec line"))?;
    let args = exec_args(&exec);
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty Exec line"))?;
    let mut command = Command::new(program);
    command.args(rest);
    if let Some(folder) = entry_value(&text, "Path").filter(|folder| !folder.is_empty()) {
        command.current_dir(folder);
    }
    super::spawn(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDITOR: &str = "# comment\n[Desktop Entry]\nType=Application\nName=Editor\nName[pt_BR]=Editor\nIcon=org.example.Editor\nExec=\"/opt/my editor/run\" --new-window %U\nPath=/opt/my editor\n\n[Desktop Action new]\nExec=other\nIcon=other\n";

    #[test]
    fn reads_keys_of_the_desktop_entry_group() {
        assert_eq!(
            entry_value(EDITOR, "Icon").as_deref(),
            Some("org.example.Editor")
        );
        assert_eq!(
            entry_value(EDITOR, "Path").as_deref(),
            Some("/opt/my editor")
        );
        assert_eq!(entry_value(EDITOR, "Terminal"), None);
        assert_eq!(
            entry_value("[Desktop Entry]\nName=A\\sB\n", "Name").as_deref(),
            Some("A B")
        );
    }

    #[test]
    fn exec_lines_become_arguments() {
        let exec = entry_value(EDITOR, "Exec").unwrap();
        assert_eq!(exec_args(&exec), ["/opt/my editor/run", "--new-window"]);
        assert_eq!(exec_args("firefox %u"), ["firefox"]);
        assert_eq!(exec_args("app --rate=50%% %F"), ["app", "--rate=50%"]);
        assert_eq!(
            exec_args(r#"sh -c "echo \"hi\" \$HOME""#),
            ["sh", "-c", r#"echo "hi" $HOME"#]
        );
        assert!(exec_args("  %f ").is_empty());
    }

    #[test]
    fn icons_are_looked_up_by_theme_and_size() {
        let bases = [
            PathBuf::from("/usr/share"),
            PathBuf::from("/home/ana/.local/share"),
        ];
        let candidates = icon_candidates("firefox.png", &bases);
        assert_eq!(
            candidates[0],
            PathBuf::from("/usr/share/icons/hicolor/48x48/apps/firefox.png")
        );
        assert_eq!(
            candidates[2],
            PathBuf::from("/usr/share/icons/hicolor/scalable/apps/firefox.svg")
        );
        assert!(candidates.contains(&PathBuf::from(
            "/home/ana/.local/share/icons/breeze/apps/48/firefox.svg"
        )));
        assert_eq!(
            candidates.last(),
            Some(&PathBuf::from("/usr/share/pixmaps/firefox.svg"))
        );
    }

    #[test]
    fn icon_bases_cover_user_flatpak_and_system() {
        let bases = icon_bases(Path::new("/home/ana"), None);
        assert_eq!(
            bases,
            [
                PathBuf::from("/home/ana/.local/share"),
                PathBuf::from("/home/ana/.local/share/flatpak/exports/share"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
                PathBuf::from("/var/lib/flatpak/exports/share"),
            ]
        );
        let custom = icon_bases(
            Path::new("/home/ana"),
            Some("/usr/share:relative:/var/lib/flatpak/exports/share"),
        );
        assert_eq!(custom.len(), 4);
    }

    #[test]
    fn large_icons_shrink_by_whole_factors() {
        let icon = IconPixels {
            width: 4,
            height: 2,
            rgba: [
                [255, 0, 0, 255],
                [0, 0, 255, 255],
                [0, 0, 0, 0],
                [9, 9, 9, 0],
            ]
            .repeat(2)
            .concat(),
        };
        let small = shrink(icon.clone(), 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(&small.rgba[..4], &[127, 0, 127, 255]);
        assert_eq!(&small.rgba[4..], &[0, 0, 0, 0]);
        assert_eq!(shrink(icon.clone(), 4), icon);
    }
}
