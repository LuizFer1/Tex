use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tex_indexer::desktop::SYSTEM_APP_DIRS;
use tex_indexer::engine::Engine;
use tex_indexer::linux::apps::read_apps;
use tex_indexer::linux::{disk, socket};
use tex_ipc::VolumeState;

pub fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: fake_server <absolute folder>");
    let started = Instant::now();
    let (index, _) = disk::scan(&root, &HashSet::new(), &mut |_| {});
    println!(
        "indexed {} entries from {root} in {:?}",
        index.len(),
        started.elapsed()
    );
    let engine = Arc::new(Engine::new());
    engine
        .apps_mut()
        .set_system(read_apps(&SYSTEM_APP_DIRS.map(String::from)));
    let volume = engine
        .add(index.root(), 0)
        .expect("a new engine has no volumes");
    {
        let mut data = volume.write();
        data.state = VolumeState::Ready {
            entries: index.len() as u32,
        };
        data.index = Some(index);
    }
    let path = tex_ipc::socket_path();
    println!("serving {path}; stop with Ctrl+C");
    socket::serve(Path::new(&path), engine)
        .expect("socket server (set TEX_INDEXER_SOCKET to a path you can write)");
}
