use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// What the UI asks of a clipboard thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    /// Own the clipboard with these targets and their bytes.
    Write(Vec<(String, Vec<u8>)>),
    Stop,
}

/// Sends [`Message`]s to a thread that sleeps in `poll`, waking it through a socket.
#[derive(Clone)]
pub(crate) struct Waker {
    sender: Sender<Message>,
    socket: Arc<UnixStream>,
}

/// The thread's end: a descriptor to poll and the messages behind it.
pub(crate) struct Inbox {
    receiver: Receiver<Message>,
    socket: UnixStream,
}

pub(crate) fn channel() -> io::Result<(Waker, Inbox)> {
    let (wake, woken) = UnixStream::pair()?;
    woken.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel();
    Ok((
        Waker {
            sender,
            socket: Arc::new(wake),
        },
        Inbox {
            receiver,
            socket: woken,
        },
    ))
}

impl Waker {
    pub(crate) fn send(&self, message: Message) -> io::Result<()> {
        self.sender
            .send(message)
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "clipboard thread stopped"))?;
        (&*self.socket).write_all(&[1])
    }
}

impl Inbox {
    pub(crate) fn fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }

    /// The messages sent so far, emptying the socket.
    pub(crate) fn take(&self) -> Vec<Message> {
        let mut buffer = [0u8; 64];
        while matches!((&self.socket).read(&mut buffer), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}

/// Waits until one of `fds` can be read or `timeout` passes (`None` waits forever). Returns which
/// ones are readable (or closed).
pub(crate) fn wait(fds: &[RawFd], timeout: Option<Duration>) -> io::Result<Vec<bool>> {
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let millis = timeout.map_or(-1, |timeout| {
        libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX)
    });
    loop {
        let ready =
            unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, millis) };
        if ready >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(polled
        .iter()
        .map(|fd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_wakes_the_inbox() {
        let (waker, inbox) = channel().unwrap();
        assert_eq!(
            wait(&[inbox.fd()], Some(Duration::from_millis(10))).unwrap(),
            [false]
        );
        waker
            .send(Message::Write(vec![("a".into(), vec![1])]))
            .unwrap();
        waker.send(Message::Stop).unwrap();
        assert_eq!(
            wait(&[inbox.fd()], Some(Duration::from_secs(1))).unwrap(),
            [true]
        );
        assert_eq!(
            inbox.take(),
            [Message::Write(vec![("a".into(), vec![1])]), Message::Stop]
        );
        assert_eq!(
            wait(&[inbox.fd()], Some(Duration::from_millis(10))).unwrap(),
            [false]
        );
    }

    #[test]
    fn a_stopped_thread_refuses_messages() {
        let (waker, inbox) = channel().unwrap();
        drop(inbox);
        assert!(waker.send(Message::Stop).is_err());
    }
}
