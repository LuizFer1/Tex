use std::fs;
use std::path::Path;

const THEME_FILES: &[&str] = &["theme.slint", "artwork.slint"];
const LEGACY: &[&str] = &["history.slint", "settings.slint", "about.slint"];

fn slint_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "slint") {
            out.push(path);
        }
    }
}

fn color_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    for (start, _) in source.match_indices('#') {
        let digits = bytes[start + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        let next = bytes.get(start + 1 + digits).copied().unwrap_or(b' ');
        if matches!(digits, 3 | 4 | 6 | 8) && !next.is_ascii_alphanumeric() {
            found.push(source[start..start + 1 + digits].to_string());
        }
    }
    found
}

#[test]
fn screens_take_colors_only_from_tokens() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    slint_files(&ui, &mut files);
    for file in files {
        let name = file.file_name().unwrap().to_str().unwrap();
        if THEME_FILES.contains(&name) || LEGACY.contains(&name) {
            continue;
        }
        let literals = color_literals(&fs::read_to_string(&file).unwrap());
        assert!(
            literals.is_empty(),
            "{name} has color literals: {literals:?}"
        );
    }
}

#[test]
fn finds_color_literals() {
    assert_eq!(color_literals("a: #2e9e3f; b: #fff;"), ["#2e9e3f", "#fff"]);
    assert!(color_literals("// see #12 and #section").is_empty());
}
