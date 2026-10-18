mod mime;
mod wake;
mod wayland;
mod x11;

use std::io;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mime::Backend;
use wake::{Message, Waker};

use crate::{Clip, ClipFormats, Payload};

type Wanted = Box<dyn Fn(&ClipFormats) -> Option<Payload> + Send>;
type OnChange = Box<dyn Fn(Clip) + Send>;
type Writer = Box<dyn Fn(Vec<(String, Vec<u8>)>) -> io::Result<()> + Send>;

/// The most a single copy may bring; larger ones are not read.
const READ_LIMIT: usize = 256 * 1024 * 1024;
/// How long an app may take to send the next bytes of a copy.
const STALL: Duration = Duration::from_secs(2);

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

enum Stop {
    /// A thread that sleeps in `poll` and stops on [`Message::Stop`].
    Thread(Waker, JoinHandle<()>),
}

/// Watches the clipboard on its own thread: X11 through XFixes, Wayland through the
/// data-control protocols, GNOME through Tex's extension. For every change `wanted` picks the
/// content to read from the formats alone and `on_change` gets it. While it runs, `write_text`
/// and `write_image` go through the same backend. Dropping it stops the thread.
pub struct ClipboardWatcher {
    backend: Backend,
    stop: Option<Stop>,
}

impl ClipboardWatcher {
    pub fn start(
        wanted: impl Fn(&ClipFormats) -> Option<Payload> + Send + 'static,
        on_change: impl Fn(Clip) + Send + 'static,
    ) -> io::Result<ClipboardWatcher> {
        let forced = if cfg!(debug_assertions) {
            std::env::var("TEX_CLIPBOARD").ok()
        } else {
            None
        };
        let wayland = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
        let candidates = mime::backends(
            forced.as_deref(),
            super::x11::is_x11(),
            wayland,
            super::shell::is_gnome(),
        );
        let wanted: Wanted = Box::new(wanted);
        let on_change: OnChange = Box::new(on_change);
        for backend in candidates {
            match backend {
                Backend::X11 => {
                    if let Ok(selection) = x11::open() {
                        let (waker, inbox) = wake::channel()?;
                        let thread = thread::spawn(move || selection.run(inbox, wanted, on_change));
                        return Ok(ClipboardWatcher::threaded(backend, waker, thread));
                    }
                }
                Backend::DataControl => {
                    if let Ok(device) = wayland::open() {
                        let (waker, inbox) = wake::channel()?;
                        let thread = thread::spawn(move || device.run(inbox, wanted, on_change));
                        return Ok(ClipboardWatcher::threaded(backend, waker, thread));
                    }
                }
                Backend::Shell => {}
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this desktop does not let apps read the clipboard",
        ))
    }

    fn threaded(backend: Backend, waker: Waker, thread: JoinHandle<()>) -> ClipboardWatcher {
        let writer = waker.clone();
        set_writer(Some(Box::new(move |items| {
            writer.send(Message::Write(items))
        })));
        ClipboardWatcher {
            backend,
            stop: Some(Stop::Thread(waker, thread)),
        }
    }

    /// Whether the copies carry the app that made them, which ignored apps need.
    pub fn knows_owner(&self) -> bool {
        self.backend != Backend::DataControl
    }

    /// Whether capture depends on Tex's GNOME Shell extension.
    pub fn through_shell(&self) -> bool {
        self.backend == Backend::Shell
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        set_writer(None);
        if let Some(Stop::Thread(waker, thread)) = self.stop.take() {
            let _ = waker.send(Message::Stop);
            let _ = thread.join();
        }
    }
}

fn set_writer(writer: Option<Writer>) {
    *WRITER.lock().unwrap_or_else(|error| error.into_inner()) = writer;
}

fn write(items: Vec<(String, Vec<u8>)>) -> io::Result<()> {
    match &*WRITER.lock().unwrap_or_else(|error| error.into_inner()) {
        Some(writer) => writer(items),
        None => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "the clipboard is not being watched",
        )),
    }
}

/// Replaces the clipboard with `text`, marked as a Tex copy.
pub fn write_text(text: &str) -> io::Result<()> {
    write(mime::text_offers(text))
}

/// Replaces the clipboard with a PNG image, marked as a Tex copy. The DIB is for Windows.
pub fn write_image(_dib: &[u8], png: &[u8]) -> io::Result<()> {
    write(mime::png_offers(png))
}
