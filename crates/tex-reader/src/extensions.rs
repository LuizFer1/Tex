use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    #[default]
    Document,
    Selection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolOutput {
    Replace,
    #[default]
    Show,
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewerOutput {
    #[default]
    Markdown,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub title: String,
    pub files: Vec<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub output: ToolOutput,
    pub shortcut: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewer {
    pub files: Vec<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub output: ViewerOutput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    #[serde(default)]
    tool: Vec<Tool>,
    #[serde(default)]
    viewer: Vec<Viewer>,
}

#[derive(Clone, Debug)]
pub struct Extension {
    pub name: String,
    pub dir: PathBuf,
    pub tools: Vec<Tool>,
    pub viewers: Vec<Viewer>,
}

#[derive(Clone, Debug)]
pub struct Broken {
    pub folder: PathBuf,
    pub error: String,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub extensions: Vec<Extension>,
    pub broken: Vec<Broken>,
}

pub const MANIFEST: &str = "extension.toml";

impl Catalog {
    /// Loads every `<folder>/extension.toml` under `dir`, folders in alphabetical order; a missing
    /// `dir` is an empty catalog and a bad manifest is reported in `broken`.
    pub fn discover(dir: &Path) -> Catalog {
        let mut folders: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.is_dir())
                .collect(),
            Err(_) => return Catalog::default(),
        };
        folders.sort();
        let mut catalog = Catalog::default();
        for folder in folders {
            match load(&folder) {
                Ok(extension) => catalog.extensions.push(extension),
                Err(error) => catalog.broken.push(Broken { folder, error }),
            }
        }
        catalog
    }

    /// Tools for a file type (`None`: no extension), in folder then manifest order.
    pub fn tools_for(&self, ext: Option<&str>) -> Vec<(&Extension, &Tool)> {
        self.extensions
            .iter()
            .flat_map(|extension| extension.tools.iter().map(move |tool| (extension, tool)))
            .filter(|(_, tool)| {
                tool.files
                    .iter()
                    .any(|file| file == "*" || Some(file.as_str()) == ext)
            })
            .collect()
    }

    /// The viewer for a type: the first folder that declares it wins.
    pub fn viewer_for(&self, ext: &str) -> Option<(&Extension, &Viewer)> {
        self.extensions.iter().find_map(|extension| {
            extension
                .viewers
                .iter()
                .find(|viewer| viewer.files.iter().any(|file| file == ext))
                .map(|viewer| (extension, viewer))
        })
    }

    /// Every type some viewer opens, sorted and without repeats.
    pub fn viewer_extensions(&self) -> Vec<String> {
        let mut exts: Vec<String> = self
            .extensions
            .iter()
            .flat_map(|extension| extension.viewers.iter())
            .flat_map(|viewer| viewer.files.iter().cloned())
            .collect();
        exts.sort();
        exts.dedup();
        exts
    }

    /// `(type, extension name)` for every viewer that lost its type to an earlier folder.
    pub fn viewer_conflicts(&self) -> Vec<(String, String)> {
        let mut seen: Vec<&str> = Vec::new();
        let mut conflicts = Vec::new();
        for extension in &self.extensions {
            for viewer in &extension.viewers {
                for file in &viewer.files {
                    if seen.contains(&file.as_str()) {
                        conflicts.push((file.clone(), extension.name.clone()));
                    } else {
                        seen.push(file);
                    }
                }
            }
        }
        conflicts
    }
}

fn load(folder: &Path) -> Result<Extension, String> {
    let text = std::fs::read_to_string(folder.join(MANIFEST)).map_err(|error| error.to_string())?;
    let manifest: Manifest = toml::from_str(&text).map_err(|error| error.to_string())?;
    if manifest.name.trim().is_empty() {
        return Err("name is empty".into());
    }
    let mut tools = manifest.tool;
    let mut viewers = manifest.viewer;
    for tool in &mut tools {
        tool.files = normalized(&tool.files, &tool.title)?;
        check_command(&tool.command, &tool.title)?;
    }
    for viewer in &mut viewers {
        viewer.files = normalized(&viewer.files, "viewer")?;
        check_command(&viewer.command, "viewer")?;
    }
    Ok(Extension {
        name: manifest.name,
        dir: folder.to_path_buf(),
        tools,
        viewers,
    })
}

fn normalized(files: &[String], owner: &str) -> Result<Vec<String>, String> {
    let files: Vec<String> = files
        .iter()
        .map(|file| file.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|file| !file.is_empty())
        .collect();
    if files.is_empty() {
        return Err(format!("{owner}: files is empty"));
    }
    Ok(files)
}

fn check_command(command: &[String], owner: &str) -> Result<(), String> {
    match command.first() {
        Some(program) if !program.trim().is_empty() => Ok(()),
        _ => Err(format!("{owner}: command is empty")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    pub input: Vec<u8>,
    pub env: Vec<(String, String)>,
}

/// The process to run for `command` of `extension` on `file`, with `input` on stdin.
pub fn job(extension: &Extension, command: &[String], input: Vec<u8>, file: &Path) -> Job {
    let local = |part: &String| {
        let path = extension.dir.join(part);
        if path.is_file() {
            path.to_string_lossy().into_owned()
        } else {
            part.clone()
        }
    };
    Job {
        program: command.first().map(local).unwrap_or_default(),
        args: command.iter().skip(1).map(local).collect(),
        dir: extension.dir.clone(),
        input,
        env: vec![
            ("TEX_FILE".into(), file.to_string_lossy().into_owned()),
            (
                "TEX_EXTENSION_DIR".into(),
                extension.dir.to_string_lossy().into_owned(),
            ),
            ("TEX_VERSION".into(), env!("CARGO_PKG_VERSION").into()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn root() -> Temp {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tex-ext-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn write(root: &Path, folder: &str, manifest: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    }

    const JSON: &str = r#"
name = "Formatar JSON"

[[tool]]
title = "Formatar"
files = ["JSON", ".jsonl"]
command = ["python", "format.py"]
input = "document"
output = "replace"
shortcut = "Ctrl+Shift+F"

[[tool]]
title = "Contar"
files = ["*"]
command = ["wc"]
"#;

    const SRT: &str = r#"
name = "Legendas"

[[viewer]]
files = ["srt", "vtt"]
command = ["python", "srt.py"]
"#;

    #[test]
    fn a_missing_folder_is_an_empty_catalog() {
        let catalog = Catalog::discover(Path::new("C:/nao/existe/tex-ext"));
        assert!(catalog.extensions.is_empty());
        assert!(catalog.broken.is_empty());
    }

    #[test]
    fn manifests_load_with_defaults_and_lowercase_types() {
        let root = root();
        write(&root.0, "json", JSON);
        let catalog = Catalog::discover(&root.0);
        let extension = &catalog.extensions[0];
        assert_eq!(extension.name, "Formatar JSON");
        assert_eq!(extension.dir, root.0.join("json"));
        assert_eq!(extension.tools[0].files, ["json", "jsonl"]);
        assert_eq!(extension.tools[0].output, ToolOutput::Replace);
        assert_eq!(extension.tools[1].input, Input::Document);
        assert_eq!(extension.tools[1].output, ToolOutput::Show);
        assert_eq!(extension.tools[1].shortcut, None);
    }

    #[test]
    fn tools_match_their_types_and_the_wildcard() {
        let root = root();
        write(&root.0, "json", JSON);
        let catalog = Catalog::discover(&root.0);
        let titles = |ext: Option<&str>| -> Vec<String> {
            catalog
                .tools_for(ext)
                .iter()
                .map(|(_, tool)| tool.title.clone())
                .collect()
        };
        assert_eq!(titles(Some("json")), ["Formatar", "Contar"]);
        assert_eq!(titles(Some("txt")), ["Contar"]);
        assert_eq!(titles(None), ["Contar"]);
    }

    #[test]
    fn the_first_folder_wins_a_viewer_type() {
        let root = root();
        write(&root.0, "b-legendas", SRT);
        write(
            &root.0,
            "a-outro",
            "name = \"Outro\"\n[[viewer]]\nfiles = [\"srt\"]\ncommand = [\"x\"]\noutput = \"text\"\n",
        );
        let catalog = Catalog::discover(&root.0);
        let (extension, viewer) = catalog.viewer_for("srt").unwrap();
        assert_eq!(extension.name, "Outro");
        assert_eq!(viewer.output, ViewerOutput::Text);
        assert_eq!(catalog.viewer_for("vtt").unwrap().0.name, "Legendas");
        assert_eq!(catalog.viewer_extensions(), ["srt", "vtt"]);
        assert_eq!(
            catalog.viewer_conflicts(),
            [("srt".to_string(), "Legendas".to_string())]
        );
    }

    #[test]
    fn broken_manifests_are_reported_not_loaded() {
        let root = root();
        write(
            &root.0,
            "ruim",
            "name = \"x\"\n[[tool]]\ntitle = \"t\"\nfiles = []\ncommand = [\"a\"]\n",
        );
        write(&root.0, "toml", "isto nao e toml");
        write(
            &root.0,
            "vazio",
            "name = \"x\"\n[[tool]]\ntitle = \"t\"\nfiles = [\"md\"]\ncommand = []\n",
        );
        std::fs::create_dir_all(root.0.join("sem-manifesto")).unwrap();
        let catalog = Catalog::discover(&root.0);
        assert!(catalog.extensions.is_empty());
        assert_eq!(catalog.broken.len(), 4);
        assert!(catalog.broken.iter().all(|broken| !broken.error.is_empty()));
    }

    #[test]
    fn jobs_run_in_the_extension_folder_with_tex_variables() {
        let root = root();
        write(&root.0, "json", JSON);
        std::fs::write(root.0.join("json").join("format.py"), "").unwrap();
        let catalog = Catalog::discover(&root.0);
        let extension = &catalog.extensions[0];
        let job = job(
            extension,
            &extension.tools[0].command,
            b"{}".to_vec(),
            Path::new("a.json"),
        );
        assert_eq!(job.dir, root.0.join("json"));
        assert_eq!(job.program, "python");
        assert_eq!(
            job.args,
            [root
                .0
                .join("json")
                .join("format.py")
                .to_string_lossy()
                .into_owned()]
        );
        assert!(
            job.env
                .iter()
                .any(|(key, value)| key == "TEX_FILE" && value == "a.json")
        );
        assert!(job.env.iter().any(|(key, _)| key == "TEX_EXTENSION_DIR"));
        assert!(job.env.iter().any(|(key, _)| key == "TEX_VERSION"));
    }
}
