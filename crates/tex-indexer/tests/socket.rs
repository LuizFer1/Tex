#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tex_index::Viewer;
use tex_indexer::engine::Handler;
use tex_indexer::linux::socket;
use tex_ipc::{Filter, Hit, Request, Response, Results, SearchQuery, read_message, write_message};

struct Echo;

impl Handler for Echo {
    fn handle(&self, request: Request) -> Response {
        self.handle_as(request, None)
    }

    fn handle_as(&self, request: Request, viewer: Option<&Viewer>) -> Response {
        let Request::Search(query) = request else {
            return Response::Status(Vec::new());
        };
        let groups = viewer.map_or(Vec::new(), |viewer| viewer.groups.clone());
        Response::Results(Results {
            id: query.id,
            hits: groups
                .into_iter()
                .map(|group| Hit {
                    path: group.to_string(),
                    is_dir: false,
                    is_app: false,
                    label: None,
                })
                .collect(),
            total: viewer.map_or(u32::MAX, |viewer| viewer.uid),
        })
    }
}

fn connect(path: &Path) -> UnixStream {
    for _ in 0..100 {
        if let Ok(stream) = UnixStream::connect(path) {
            return stream;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("could not connect to {}", path.display());
}

fn search() -> Request {
    Request::Search(SearchQuery {
        id: 3,
        query: "x".into(),
        filter: Filter::All,
        limit: 5,
        home: String::new(),
        include_hidden: false,
        include_system: false,
        excluded_folders: Vec::new(),
        show_apps: true,
        show_files: true,
        show_folders: true,
        language: String::new(),
    })
}

#[test]
fn answers_each_client_as_the_user_who_connected() {
    let path = std::env::temp_dir().join(format!("tex-socket-{}.sock", std::process::id()));
    let server_path = path.clone();
    thread::spawn(move || socket::serve(&server_path, Arc::new(Echo)));
    let stream = connect(&path);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o666
    );
    write_message(&mut &stream, &search()).unwrap();
    let Response::Results(results) = read_message(&mut &stream).unwrap() else {
        panic!("expected results");
    };
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    assert_eq!(results.id, 3);
    assert_eq!(results.total, uid);
    assert_eq!(results.hits[0].path, gid.to_string());
    let error = socket::serve(&path, Arc::new(Echo)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    drop(stream);
    let _ = fs::remove_file(&path);
}
