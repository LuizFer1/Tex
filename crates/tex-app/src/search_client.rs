use std::fs::{File, OpenOptions};
use std::io;
use std::iter;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use tex_ipc::{
    Filter, PIPE_NAME, Request, Response, Results, SearchQuery, VolumeStatus, read_message,
    write_message,
};

const LIMIT: u16 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Search { query: String, filter: Filter },
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Results(Results),
    Status(Vec<VolumeStatus>),
    Unavailable,
}

#[derive(Clone)]
pub struct SearchClient {
    commands: Sender<Command>,
}

impl SearchClient {
    /// Starts the thread that talks to the search service. `on_event` runs on that thread.
    pub fn start(home: String, on_event: impl Fn(Event) + Send + 'static) -> Self {
        let (commands, receiver) = mpsc::channel();
        thread::spawn(move || run(&receiver, &home, &on_event));
        Self { commands }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

fn run(receiver: &Receiver<Command>, home: &str, on_event: &dyn Fn(Event)) {
    let mut connection = None;
    let mut next_id = 0;
    while let Ok(first) = receiver.recv() {
        for command in coalesce(iter::once(first).chain(receiver.try_iter())) {
            let request = match command {
                Command::Search { query, filter } => {
                    next_id += 1;
                    Request::Search(SearchQuery {
                        id: next_id,
                        query,
                        filter,
                        limit: LIMIT,
                        home: home.to_owned(),
                        include_hidden: false,
                    })
                }
                Command::Status => Request::Status,
            };
            on_event(match exchange(&mut connection, &request) {
                Ok(Response::Results(results)) => Event::Results(results),
                Ok(Response::Status(volumes)) => Event::Status(volumes),
                Err(_) => Event::Unavailable,
            });
        }
    }
}

/// Keeps one Status (if any) followed by only the newest Search, so a burst of keystrokes costs one query.
pub fn coalesce(commands: impl Iterator<Item = Command>) -> Vec<Command> {
    let mut status = false;
    let mut search = None;
    for command in commands {
        match command {
            Command::Status => status = true,
            newer => search = Some(newer),
        }
    }
    status
        .then_some(Command::Status)
        .into_iter()
        .chain(search)
        .collect()
}

/// Sends one request and waits for the answer, reconnecting once if the pipe broke
/// (for example after the service restarted).
fn exchange(connection: &mut Option<File>, request: &Request) -> io::Result<Response> {
    let mut retried = false;
    loop {
        if connection.is_none() {
            *connection = Some(OpenOptions::new().read(true).write(true).open(PIPE_NAME)?);
        }
        let file = connection.as_ref().expect("connected above");
        match write_message(&mut &*file, request).and_then(|()| read_message(&mut &*file)) {
            Ok(response) => return Ok(response),
            Err(error) => {
                *connection = None;
                if retried {
                    return Err(error);
                }
                retried = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(text: &str) -> Command {
        Command::Search {
            query: text.into(),
            filter: Filter::All,
        }
    }

    #[test]
    fn keeps_only_the_newest_search() {
        let commands = vec![search("r"), search("re"), search("rel")];
        assert_eq!(coalesce(commands.into_iter()), [search("rel")]);
    }

    #[test]
    fn status_goes_before_the_search() {
        let commands = vec![search("a"), Command::Status, search("ab"), Command::Status];
        assert_eq!(
            coalesce(commands.into_iter()),
            [Command::Status, search("ab")]
        );
    }

    #[test]
    fn single_commands_pass_through() {
        assert_eq!(
            coalesce(vec![Command::Status].into_iter()),
            [Command::Status]
        );
        assert_eq!(coalesce(vec![search("x")].into_iter()), [search("x")]);
    }
}
