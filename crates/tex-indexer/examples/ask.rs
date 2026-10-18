//! Development tool: asks the search service for its status, searches as the current user, or
//! asks it to press Ctrl+V for the current user (`--paste`: prints Sent, Refused or
//! Unavailable). Debug builds use `TEX_INDEXER_SOCKET` when set. `--wait N` waits up to N
//! seconds for the socket (and, for status and search, for every volume to finish indexing).
//! Usage: cargo run -p tex-indexer --example ask -- [--wait 60] [--paste | query]

#[cfg(unix)]
fn main() {
    use std::os::unix::net::UnixStream;
    use std::thread;
    use std::time::{Duration, Instant};

    use tex_ipc::{
        Filter, Request, Response, SearchQuery, VolumeState, read_message, write_message,
    };

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut wait = 0;
    if args.first().map(String::as_str) == Some("--wait") {
        wait = args
            .get(1)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        args.drain(..args.len().min(2));
    }
    let path = tex_ipc::socket_path();
    let deadline = Instant::now() + Duration::from_secs(wait);
    let ask = |request: &Request| -> std::io::Result<Response> {
        let stream = UnixStream::connect(&path)?;
        write_message(&mut &stream, request)?;
        read_message(&mut &stream)
    };
    if args.first().map(String::as_str) == Some("--paste") {
        loop {
            match ask(&Request::Paste) {
                Ok(Response::Paste(outcome)) => {
                    println!("{outcome:?}");
                    return;
                }
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(500)),
                other => {
                    eprintln!("ask: unexpected answer {other:?}");
                    std::process::exit(1);
                }
            }
        }
    }
    let volumes = loop {
        match ask(&Request::Status) {
            Ok(Response::Status(volumes))
                if Instant::now() >= deadline
                    || !volumes
                        .iter()
                        .any(|volume| matches!(volume.state, VolumeState::Indexing { .. })) =>
            {
                break volumes;
            }
            Err(error) if Instant::now() >= deadline => {
                eprintln!("ask: {path}: {error}");
                std::process::exit(1);
            }
            _ => thread::sleep(Duration::from_millis(500)),
        }
    };
    if args.is_empty() {
        for volume in volumes {
            println!("{} {:?}", volume.root, volume.state);
        }
        return;
    }
    let request = Request::Search(SearchQuery {
        id: 1,
        query: args.join(" "),
        filter: Filter::All,
        limit: 20,
        home: std::env::var("HOME").unwrap_or_default(),
        include_hidden: false,
        include_system: false,
        excluded_folders: Vec::new(),
        show_apps: true,
        show_files: true,
        show_folders: true,
        language: String::new(),
    });
    match ask(&request) {
        Ok(Response::Results(results)) => {
            println!("{} results", results.total);
            for hit in results.hits {
                match hit.label {
                    Some(label) => println!("{} ({label})", hit.path),
                    None => println!("{}", hit.path),
                }
            }
        }
        other => {
            eprintln!("ask: unexpected answer {other:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("ask: only on Linux; on Windows use the app");
}
