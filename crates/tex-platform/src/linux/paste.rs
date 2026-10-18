use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::Duration;

use tex_ipc::{PasteOutcome, Request, Response, read_message, write_message};

/// Time for the window that gets the focus back to have it before the keys arrive.
const FOCUS_DELAY: Duration = Duration::from_millis(150);

/// Asks the search service on a thread to press Ctrl+V, once the focus had time to come back.
/// Without the service, or when it does not paste, the content stays only on the clipboard.
pub(crate) fn request() {
    thread::spawn(|| {
        thread::sleep(FOCUS_DELAY);
        match ask(Path::new(&tex_ipc::socket_path())) {
            Ok(PasteOutcome::Sent) => {}
            Ok(outcome) => eprintln!("paste: the search service answered {outcome:?}"),
            Err(error) => eprintln!("paste: {error}"),
        }
    });
}

/// Sends `Paste` to the service listening at `path` and returns what it did.
pub(crate) fn ask(path: &Path) -> io::Result<PasteOutcome> {
    let stream = UnixStream::connect(path)?;
    write_message(&mut &stream, &Request::Paste)?;
    match read_message(&mut &stream)? {
        Response::Paste(outcome) => Ok(outcome),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected answer {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread::JoinHandle;

    use super::*;

    fn socket(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("tex-paste-{name}-{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        path
    }

    /// Answers one request with `answer` and hands back the request it got.
    fn serve_once(path: &Path, answer: Response) -> JoinHandle<Request> {
        let listener = UnixListener::bind(path).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let request: Request = read_message(&mut &stream).unwrap();
            write_message(&mut &stream, &answer).unwrap();
            request
        })
    }

    #[test]
    fn asks_the_service_to_paste() {
        let path = socket("sent");
        let server = serve_once(&path, Response::Paste(PasteOutcome::Sent));
        assert_eq!(ask(&path).unwrap(), PasteOutcome::Sent);
        assert_eq!(server.join().unwrap(), Request::Paste);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn passes_a_refusal_on() {
        let path = socket("refused");
        let server = serve_once(&path, Response::Paste(PasteOutcome::Refused));
        assert_eq!(ask(&path).unwrap(), PasteOutcome::Refused);
        server.join().unwrap();
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn another_answer_is_an_error() {
        let path = socket("other");
        let server = serve_once(&path, Response::Status(Vec::new()));
        assert_eq!(ask(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        server.join().unwrap();
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn without_the_service_it_fails() {
        let path = socket("missing");
        assert!(ask(&path).is_err());
    }
}
