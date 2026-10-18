#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tex_indexer::engine::Handler;
use tex_indexer::{account, pipe};
use tex_ipc::{Filter, Request, Response, Results, SearchQuery, read_message, write_message};

struct Fixed;

impl Handler for Fixed {
    fn handle(&self, request: Request) -> Response {
        match request {
            Request::Search(query) => Response::Results(Results {
                id: query.id,
                hits: Vec::new(),
                total: 7,
            }),
            Request::Status => Response::Status(Vec::new()),
        }
    }
}

fn connect(name: &str) -> File {
    for _ in 0..100 {
        if let Ok(file) = OpenOptions::new().read(true).write(true).open(name) {
            return file;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("could not connect to {name}");
}

fn ask(file: &File, request: &Request) -> Response {
    write_message(&mut &*file, request).unwrap();
    read_message(&mut &*file).unwrap()
}

#[test]
fn serves_several_clients_over_a_named_pipe() {
    let name = format!(r"\\.\pipe\tex-test-{}", std::process::id());
    let sddl = pipe::service_sddl(&pipe::PipeAccess::User(
        account::current_user_sid().unwrap(),
    ));
    let server_name = name.clone();
    thread::spawn(move || pipe::serve(&server_name, &sddl, Arc::new(Fixed)));

    let first = connect(&name);
    let second = connect(&name);
    assert_eq!(ask(&first, &Request::Status), Response::Status(Vec::new()));
    let search = Request::Search(SearchQuery {
        id: 9,
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
    });
    assert_eq!(
        ask(&second, &search),
        Response::Results(Results {
            id: 9,
            hits: Vec::new(),
            total: 7
        })
    );
    assert_eq!(
        ask(&first, &search),
        Response::Results(Results {
            id: 9,
            hits: Vec::new(),
            total: 7
        })
    );
}

#[test]
fn current_user_sid_looks_like_a_sid() {
    assert!(account::current_user_sid().unwrap().starts_with("S-1-5-"));
}
