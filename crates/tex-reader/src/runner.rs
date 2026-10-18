use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::extensions::Job;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub time: Duration,
    pub output: usize,
}

pub const LIMITS: Limits = Limits {
    time: Duration::from_secs(10),
    output: 16 * 1024 * 1024,
};

#[derive(Debug)]
pub enum RunError {
    Spawn(io::Error),
    Timeout,
    TooMuchOutput,
    Cancelled,
    InvalidUtf8,
    Failed { code: Option<i32>, stderr: String },
}

pub enum Message {
    Stdout(Option<Vec<u8>>),
    Stderr(Vec<u8>),
    Cancel,
}

/// Stops a running job from another thread.
#[derive(Clone)]
pub struct Cancel(Sender<Message>);

impl Cancel {
    pub fn cancel(&self) {
        let _ = self.0.send(Message::Cancel);
    }
}

pub struct Inbox {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

pub fn channel() -> (Cancel, Inbox) {
    let (sender, receiver) = mpsc::channel();
    (Cancel(sender.clone()), Inbox { sender, receiver })
}

/// Runs `job` and returns its stdout as UTF-8. One thread writes stdin and two read the outputs;
/// this thread only waits on the channel, so a timeout, a cancel or too much output kills the process.
pub fn run(job: &Job, limits: Limits, inbox: Inbox) -> Result<String, RunError> {
    let mut command = Command::new(&job.program);
    command
        .args(&job.args)
        .current_dir(&job.dir)
        .envs(job.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let mut stdin = child.stdin.take();
    let input = job.input.clone();
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&input);
        }
    });
    let stdout = child.stdout.take();
    let sender = inbox.sender.clone();
    let limit = limits.output;
    std::thread::spawn(move || {
        let data = stdout.and_then(|stdout| read_limited(stdout, limit));
        let _ = sender.send(Message::Stdout(data));
    });
    let stderr = child.stderr.take();
    let sender = inbox.sender.clone();
    std::thread::spawn(move || {
        let data = stderr
            .and_then(|stderr| read_limited(stderr, 64 * 1024))
            .unwrap_or_default();
        let _ = sender.send(Message::Stderr(data));
    });
    drop(inbox.sender);
    let deadline = Instant::now() + limits.time;
    let mut out = None;
    let mut err = None;
    while out.is_none() || err.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match inbox.receiver.recv_timeout(left) {
            Ok(Message::Stdout(Some(data))) => out = Some(data),
            Ok(Message::Stdout(None)) => return Err(kill(&mut child, RunError::TooMuchOutput)),
            Ok(Message::Stderr(data)) => err = Some(data),
            Ok(Message::Cancel) => return Err(kill(&mut child, RunError::Cancelled)),
            Err(RecvTimeoutError::Timeout) => return Err(kill(&mut child, RunError::Timeout)),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(RunError::Spawn)?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&err.unwrap_or_default())
            .trim()
            .to_string();
        return Err(RunError::Failed {
            code: status.code(),
            stderr,
        });
    }
    String::from_utf8(out.unwrap_or_default()).map_err(|_| RunError::InvalidUtf8)
}

/// Reads to the end; `None` when more than `limit` bytes arrive.
fn read_limited(reader: impl Read, limit: usize) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    let read = reader.take(limit as u64 + 1).read_to_end(&mut data).ok()?;
    (read <= limit).then_some(data)
}

fn kill(child: &mut Child, error: RunError) -> RunError {
    let _ = child.kill();
    let _ = child.wait();
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn job(program: &str, args: &[&str], input: &[u8]) -> Job {
        Job {
            program: program.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            dir: std::env::temp_dir(),
            input: input.to_vec(),
            env: vec![("TEX_FILE".into(), "x.txt".into())],
        }
    }

    #[cfg(windows)]
    fn upper() -> Job {
        job(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[Console]::In.ReadToEnd().ToUpper()",
            ],
            b"ola",
        )
    }
    #[cfg(not(windows))]
    fn upper() -> Job {
        job("tr", &["a-z", "A-Z"], b"ola")
    }

    #[cfg(windows)]
    fn failing() -> Job {
        job("cmd", &["/c", "echo deu errado 1>&2 & exit /b 3"], b"")
    }
    #[cfg(not(windows))]
    fn failing() -> Job {
        job("sh", &["-c", "echo deu errado >&2; exit 3"], b"")
    }

    #[cfg(windows)]
    fn sleeping() -> Job {
        job(
            "powershell",
            &["-NoProfile", "-Command", "Start-Sleep -Seconds 20"],
            b"",
        )
    }
    #[cfg(not(windows))]
    fn sleeping() -> Job {
        job("sleep", &["20"], b"")
    }

    #[cfg(windows)]
    fn chatty() -> Job {
        job(
            "cmd",
            &[
                "/c",
                "for /l %i in (1,1,5000) do @echo xxxxxxxxxxxxxxxxxxxx",
            ],
            b"",
        )
    }
    #[cfg(not(windows))]
    fn chatty() -> Job {
        job(
            "sh",
            &["-c", "yes xxxxxxxxxxxxxxxxxxxx | head -n 5000"],
            b"",
        )
    }

    #[cfg(windows)]
    fn env_echo() -> Job {
        job("cmd", &["/c", "echo %TEX_FILE%"], b"")
    }
    #[cfg(not(windows))]
    fn env_echo() -> Job {
        job("sh", &["-c", "echo $TEX_FILE"], b"")
    }

    const LIMITS: Limits = Limits {
        time: Duration::from_secs(15),
        output: 1 << 20,
    };

    #[test]
    fn stdin_goes_in_and_stdout_comes_back() {
        let (_cancel, inbox) = channel();
        let output = run(&upper(), LIMITS, inbox).unwrap();
        assert_eq!(output.trim(), "OLA");
    }

    #[test]
    fn variables_reach_the_process() {
        let (_cancel, inbox) = channel();
        assert_eq!(run(&env_echo(), LIMITS, inbox).unwrap().trim(), "x.txt");
    }

    #[test]
    fn a_failing_command_reports_code_and_stderr() {
        let (_cancel, inbox) = channel();
        match run(&failing(), LIMITS, inbox) {
            Err(RunError::Failed { code, stderr }) => {
                assert_eq!(code, Some(3));
                assert!(stderr.contains("deu errado"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_slow_command_is_killed_at_the_limit() {
        let (_cancel, inbox) = channel();
        let limits = Limits {
            time: Duration::from_millis(500),
            ..LIMITS
        };
        assert!(matches!(
            run(&sleeping(), limits, inbox),
            Err(RunError::Timeout)
        ));
    }

    #[test]
    fn too_much_output_is_refused() {
        let (_cancel, inbox) = channel();
        let limits = Limits {
            output: 1000,
            ..LIMITS
        };
        assert!(matches!(
            run(&chatty(), limits, inbox),
            Err(RunError::TooMuchOutput)
        ));
    }

    #[test]
    fn cancel_kills_the_process() {
        let (cancel, inbox) = channel();
        let handle = std::thread::spawn(move || run(&sleeping(), LIMITS, inbox));
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
        assert!(matches!(handle.join().unwrap(), Err(RunError::Cancelled)));
    }

    #[test]
    fn a_missing_program_cannot_start() {
        let (_cancel, inbox) = channel();
        assert!(matches!(
            run(&job("tex-programa-que-nao-existe", &[], b""), LIMITS, inbox),
            Err(RunError::Spawn(_))
        ));
    }
}
