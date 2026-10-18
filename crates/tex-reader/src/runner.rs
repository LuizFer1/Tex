use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
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

/// How much of stderr is kept; the rest is read and dropped.
const STDERR_KEPT: usize = 64 * 1024;

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
    Exited(io::Result<ExitStatus>),
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

/// Runs `job` and returns its stdout as UTF-8. Threads write stdin, read both outputs and wait for
/// the exit; this thread only waits on the channel, so a timeout, a cancel or too much output stops
/// the whole process tree at any point, also after the outputs closed.
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let tree = match Tree::adopt(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError::Spawn(error));
        }
    };
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
            .map(|stderr| read_capped(stderr, STDERR_KEPT))
            .unwrap_or_default();
        let _ = sender.send(Message::Stderr(data));
    });
    let sender = inbox.sender;
    std::thread::spawn(move || {
        let _ = sender.send(Message::Exited(child.wait()));
    });
    let deadline = Instant::now() + limits.time;
    let mut out = None;
    let mut err = None;
    let mut status = None;
    while out.is_none() || err.is_none() || status.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        let stop = match inbox.receiver.recv_timeout(left) {
            Ok(Message::Stdout(Some(data))) => {
                out = Some(data);
                continue;
            }
            Ok(Message::Stderr(data)) => {
                err = Some(data);
                continue;
            }
            Ok(Message::Exited(Ok(exit))) => {
                status = Some(exit);
                continue;
            }
            Ok(Message::Exited(Err(error))) => RunError::Spawn(error),
            Ok(Message::Stdout(None)) => RunError::TooMuchOutput,
            Ok(Message::Cancel) => RunError::Cancelled,
            Err(RecvTimeoutError::Timeout) => RunError::Timeout,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        tree.stop();
        return Err(stop);
    }
    tree.release();
    let Some(status) = status else {
        return Err(RunError::Spawn(io::Error::other("the process was lost")));
    };
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

/// Reads to the end keeping only the first `limit` bytes, so the writer never blocks on a full pipe.
fn read_capped(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let room = limit.saturating_sub(data.len());
                data.extend_from_slice(&buffer[..read.min(room)]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    data
}

/// Every process a job starts: a job object on Windows, which also ends them if Tex dies while it
/// runs, and a process group on Unix. `stop` ends them all; `release` leaves alone whatever a job
/// that finished left running.
#[cfg(windows)]
struct Tree {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Tree {
    fn adopt(child: &Child) -> io::Result<Tree> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let tree = Tree { job };
        tree.limit(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
        if unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(tree)
    }

    fn limit(&self, flags: u32) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = flags;
        let done = unsafe {
            SetInformationJobObject(
                self.job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn stop(&self) {
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1) };
    }

    fn release(&self) {
        let _ = self.limit(0);
    }
}

#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.job) };
    }
}

#[cfg(unix)]
struct Tree {
    group: i32,
}

#[cfg(unix)]
impl Tree {
    fn adopt(child: &Child) -> io::Result<Tree> {
        let group = i32::try_from(child.id()).map_err(io::Error::other)?;
        Ok(Tree { group })
    }

    fn stop(&self) {
        unsafe { libc::kill(-self.group, libc::SIGKILL) };
    }

    fn release(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    #[cfg(windows)]
    fn noisy_stderr() -> Job {
        job(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[Console]::Error.Write('e' * 200000); [Console]::Error.Flush(); [Console]::Out.Write('fim'); exit 2",
            ],
            b"",
        )
    }
    #[cfg(not(windows))]
    fn noisy_stderr() -> Job {
        job(
            "sh",
            &[
                "-c",
                "head -c 200000 /dev/zero | tr '\\0' e >&2; printf fim; exit 2",
            ],
            b"",
        )
    }

    /// A parent that sleeps after starting a child that marks when it starts and, `delay` seconds
    /// later, that it is still alive.
    #[cfg(windows)]
    fn family(marks: &Marks, delay: u64) -> Job {
        let mut job = job(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "Start-Process -WindowStyle Hidden powershell -ArgumentList '-NoProfile','-Command','Set-Content $env:TEX_STARTED x; Start-Sleep $env:TEX_DELAY; Set-Content $env:TEX_SURVIVED x'; Start-Sleep 30",
            ],
            b"",
        );
        job.env.extend(marks.env());
        job.env.push(("TEX_DELAY".into(), delay.to_string()));
        job
    }
    #[cfg(not(windows))]
    fn family(marks: &Marks, delay: u64) -> Job {
        let mut job = job(
            "sh",
            &[
                "-c",
                "(touch \"$TEX_STARTED\"; sleep \"$TEX_DELAY\"; touch \"$TEX_SURVIVED\") & sleep 30",
            ],
            b"",
        );
        job.env.extend(marks.env());
        job.env.push(("TEX_DELAY".into(), delay.to_string()));
        job
    }

    /// A process that closes its stdout and stderr and keeps running.
    #[cfg(unix)]
    fn closes_outputs() -> Job {
        job("sh", &["-c", "exec 1>&- 2>&-; sleep 20"], b"")
    }

    struct Marks(PathBuf);

    impl Marks {
        fn new() -> Marks {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "tex-run-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Marks(dir)
        }

        fn env(&self) -> [(String, String); 2] {
            let path = |name: &str| self.0.join(name).to_string_lossy().into_owned();
            [
                ("TEX_STARTED".into(), path("started")),
                ("TEX_SURVIVED".into(), path("survived")),
            ]
        }

        fn started(&self) -> bool {
            self.0.join("started").exists()
        }

        fn survived(&self) -> bool {
            self.0.join("survived").exists()
        }
    }

    impl Drop for Marks {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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
    fn a_long_stderr_is_cut_but_read_to_the_end() {
        let (_cancel, inbox) = channel();
        match run(&noisy_stderr(), LIMITS, inbox) {
            Err(RunError::Failed { code, stderr }) => {
                assert_eq!(code, Some(2));
                assert_eq!(stderr.len(), STDERR_KEPT);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn read_capped_keeps_the_start() {
        let data = vec![7u8; 20_000];
        assert_eq!(read_capped(&data[..], 100), vec![7u8; 100]);
        assert_eq!(read_capped(&b"abc"[..], 100), b"abc");
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
    fn cancel_kills_the_children_too() {
        let marks = Marks::new();
        let (cancel, inbox) = channel();
        let process = family(&marks, 3);
        let handle = std::thread::spawn(move || run(&process, LIMITS, inbox));
        let waited = Instant::now();
        while !marks.started() && waited.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(marks.started());
        let cancelled = Instant::now();
        cancel.cancel();
        assert!(matches!(handle.join().unwrap(), Err(RunError::Cancelled)));
        assert!(cancelled.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_secs(4));
        assert!(!marks.survived());
    }

    #[test]
    fn a_timeout_kills_the_children_too() {
        let marks = Marks::new();
        let (_cancel, inbox) = channel();
        let limits = Limits {
            time: Duration::from_secs(3),
            ..LIMITS
        };
        let started = Instant::now();
        assert!(matches!(
            run(&family(&marks, 6), limits, inbox),
            Err(RunError::Timeout)
        ));
        assert!(started.elapsed() < limits.time + Duration::from_secs(5));
        if marks.started() {
            std::thread::sleep(Duration::from_secs(7));
            assert!(!marks.survived());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_closes_its_outputs_still_times_out() {
        let (_cancel, inbox) = channel();
        let limits = Limits {
            time: Duration::from_secs(1),
            ..LIMITS
        };
        let started = Instant::now();
        assert!(matches!(
            run(&closes_outputs(), limits, inbox),
            Err(RunError::Timeout)
        ));
        assert!(started.elapsed() < limits.time + Duration::from_secs(5));
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
