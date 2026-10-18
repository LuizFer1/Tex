use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Instant;

use tex_index::{Attributes, ROOT, Record, VolumeIndex};
use tex_indexer::engine::Engine;
use tex_indexer::{account, pipe};
use tex_ipc::{PIPE_NAME, VolumeState};

const HIDDEN: u32 = 0x2;
const SYSTEM: u32 = 0x4;
const REPARSE_POINT: u32 = 0x400;

pub fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: fake_server <absolute folder>");
    let started = Instant::now();
    let index = walk(Path::new(&root));
    println!(
        "indexed {} entries from {root} in {:?}",
        index.len(),
        started.elapsed()
    );

    let engine = Arc::new(Engine::new());
    let volume = engine
        .add(index.letter(), 0)
        .expect("a new engine has no volumes");
    {
        let mut data = volume.write();
        data.state = VolumeState::Ready {
            entries: index.len() as u32,
        };
        data.index = Some(index);
    }
    let sid = account::current_user_sid().expect("current user SID");
    println!("serving {PIPE_NAME}; stop with Ctrl+C");
    pipe::serve(
        PIPE_NAME,
        &pipe::service_sddl(&pipe::PipeAccess::User(sid)),
        engine,
    )
    .expect("pipe server (is the service running?)");
}

fn walk(root: &Path) -> VolumeIndex {
    let letter = root
        .to_str()
        .and_then(|path| path.chars().next())
        .filter(char::is_ascii_alphabetic)
        .expect("absolute path like C:\\Users\\you");
    let mut index = VolumeIndex::new(letter);
    let mut next: Record = 16;
    let mut parent = ROOT;
    for component in root.components() {
        if let Component::Normal(name) = component {
            index.upsert(
                next,
                parent,
                &name.to_string_lossy(),
                Attributes {
                    dir: true,
                    ..Attributes::default()
                },
            );
            parent = next;
            next += 1;
        }
    }
    let mut pending = vec![(root.to_path_buf(), parent)];
    while let Some((dir, record)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let raw = metadata.file_attributes();
            let attributes = Attributes {
                dir: metadata.is_dir(),
                hidden: raw & HIDDEN != 0,
                system: raw & SYSTEM != 0,
            };
            index.upsert(
                next,
                record,
                &entry.file_name().to_string_lossy(),
                attributes,
            );
            if metadata.is_dir() && raw & REPARSE_POINT == 0 {
                pending.push((entry.path(), next));
            }
            next += 1;
        }
    }
    index
}
