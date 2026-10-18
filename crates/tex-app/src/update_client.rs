use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use tex_platform::{http_get, open_locked};

use crate::update::{self, InstallKind, Release, Version, parse_signature, verify};

const API_LIMIT: u64 = 1024 * 1024;
const SIGNATURE_LIMIT: u64 = 1024;
const INSTALLER_LIMIT: u64 = 200 * 1024 * 1024;
const RECHECK_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const INSTALLER_ARGS: [&str; 4] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    Ready(String),
    Installing(String),
    Failed,
}

impl UpdateStatus {
    /// The state number the settings window shows and the version it names.
    pub fn parts(&self) -> (i32, &str) {
        match self {
            UpdateStatus::Idle => (0, ""),
            UpdateStatus::Checking => (1, ""),
            UpdateStatus::UpToDate => (2, ""),
            UpdateStatus::Downloading(version) => (3, version),
            UpdateStatus::Ready(version) => (4, version),
            UpdateStatus::Failed => (5, ""),
            UpdateStatus::Installing(version) => (6, version),
        }
    }

    pub fn busy(&self) -> bool {
        matches!(
            self,
            UpdateStatus::Checking | UpdateStatus::Downloading(_) | UpdateStatus::Installing(_)
        )
    }

    pub fn ready_version(&self) -> Option<&str> {
        match self {
            UpdateStatus::Ready(version) => Some(version),
            _ => None,
        }
    }
}

pub fn is_stale(checked_at: Option<Instant>, now: Instant) -> bool {
    checked_at.is_none_or(|at| now.saturating_duration_since(at) >= RECHECK_AFTER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Check,
    Apply,
}

#[derive(Clone)]
pub struct UpdateClient {
    commands: Sender<Command>,
}

impl UpdateClient {
    /// Checks and installs on a worker thread, keeping downloads in `folder`. `on_status` runs
    /// there. A copy that no package installed (a Linux build without `install-kind`) never
    /// goes online: every check finds it up to date.
    pub fn start(folder: PathBuf, on_status: impl Fn(UpdateStatus) + Send + 'static) -> Self {
        let (commands, receiver) = mpsc::channel();
        thread::spawn(move || {
            let Some(kind) = InstallKind::current() else {
                run_without_updates(&receiver, &on_status);
                return;
            };
            let checker = Checker {
                api: latest_url(&update::repository()),
                folder,
                kind,
                current: Version::current(),
                public_key: update::PUBLIC_KEY,
                fetch: http_get,
            };
            run(&receiver, &checker, &on_status);
        });
        Self { commands }
    }

    pub fn check(&self) {
        let _ = self.commands.send(Command::Check);
    }

    pub fn apply(&self) {
        let _ = self.commands.send(Command::Apply);
    }
}

fn latest_url(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases/latest")
}

fn run<F: Fn(&str, u64) -> io::Result<Vec<u8>>>(
    receiver: &Receiver<Command>,
    checker: &Checker<F>,
    emit: &dyn Fn(UpdateStatus),
) {
    let mut ready = None;
    for command in receiver {
        let status = match command {
            Command::Check => {
                emit(UpdateStatus::Checking);
                match checker.check(emit) {
                    Ok(found) => {
                        let status = found.as_ref().map_or(UpdateStatus::UpToDate, |found| {
                            UpdateStatus::Ready(found.version.to_string())
                        });
                        ready = found;
                        status
                    }
                    Err(error) => {
                        eprintln!("update: {error}");
                        ready = None;
                        UpdateStatus::Failed
                    }
                }
            }
            Command::Apply => {
                let Some(found) = &ready else {
                    continue;
                };
                let status = checker.install(found, emit);
                if status == UpdateStatus::Failed {
                    ready = None;
                }
                status
            }
        };
        emit(status);
    }
}

/// Answers every check with "up to date" without going online.
fn run_without_updates(receiver: &Receiver<Command>, emit: &dyn Fn(UpdateStatus)) {
    for command in receiver {
        if command == Command::Check {
            emit(UpdateStatus::UpToDate);
        }
    }
}

struct Checker<F> {
    api: String,
    folder: PathBuf,
    kind: InstallKind,
    current: Version,
    public_key: [u8; 32],
    fetch: F,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Ready {
    version: Version,
    installer: PathBuf,
    signature: PathBuf,
}

impl<F: Fn(&str, u64) -> io::Result<Vec<u8>>> Checker<F> {
    /// Finds a newer release and downloads its package of this install kind, or reuses one
    /// already downloaded and signed; `Ok(None)` when there is nothing to install. Removes
    /// every other download once a newer release is found, and all of them when Tex is up to
    /// date.
    fn check(&self, emit: &dyn Fn(UpdateStatus)) -> io::Result<Option<Ready>> {
        let answer = (self.fetch)(&self.api, API_LIMIT)?;
        let latest = Release::version_in(&answer).ok_or_else(|| invalid("not a release"))?;
        if latest <= self.current {
            remove_installers(&self.folder, self.kind.prefix(), None);
            return Ok(None);
        }
        let Some(release) = Release::from_json(&answer, self.kind) else {
            return Ok(None);
        };
        let name = self.kind.asset_name(release.version);
        let ready = Ready {
            version: release.version,
            installer: self.folder.join(&name),
            signature: self.folder.join(format!("{name}.sig")),
        };
        remove_installers(&self.folder, self.kind.prefix(), Some(&name));
        if self.verified(&ready) {
            return Ok(Some(ready));
        }
        let _ = fs::remove_file(&ready.installer);
        let _ = fs::remove_file(&ready.signature);
        emit(UpdateStatus::Downloading(release.version.to_string()));
        if release.installer.size > INSTALLER_LIMIT {
            return Err(invalid("the installer is too large"));
        }
        let signature = (self.fetch)(&release.signature.url, SIGNATURE_LIMIT)?;
        let installer = (self.fetch)(&release.installer.url, release.installer.size)?;
        let valid = installer.len() as u64 == release.installer.size
            && parse_signature(&signature).is_some_and(|signature| {
                verify(
                    &installer,
                    &name,
                    release.version,
                    &signature,
                    &self.public_key,
                )
            });
        if !valid {
            return Err(invalid("the installer does not match its signature"));
        }
        fs::create_dir_all(&self.folder)?;
        save(&ready.signature, &signature)?;
        save(&ready.installer, &installer)?;
        Ok(Some(ready))
    }

    fn verified(&self, ready: &Ready) -> bool {
        let (Ok(installer), Ok(signature)) =
            (fs::read(&ready.installer), fs::read(&ready.signature))
        else {
            return false;
        };
        let name = self.kind.asset_name(ready.version);
        parse_signature(&signature).is_some_and(|signature| {
            verify(
                &installer,
                &name,
                ready.version,
                &signature,
                &self.public_key,
            )
        })
    }

    /// Verifies the installer again while holding it open without write sharing, so nothing
    /// can swap it before Windows elevates it, then runs it and waits. Setup closes Tex before
    /// installing, so returning means it did not install (UAC cancelled or setup failed).
    fn install(&self, ready: &Ready, emit: &dyn Fn(UpdateStatus)) -> UpdateStatus {
        let Ok(mut file) = open_locked(&ready.installer) else {
            return UpdateStatus::Failed;
        };
        if !self.verified_open(&mut file, ready) {
            drop(file);
            remove_installers(&self.folder, self.kind.prefix(), None);
            return UpdateStatus::Failed;
        }
        let version = ready.version.to_string();
        emit(UpdateStatus::Installing(version.clone()));
        let status = match process::Command::new(&ready.installer)
            .args(INSTALLER_ARGS)
            .spawn()
        {
            Ok(mut child) => {
                let _ = child.wait();
                UpdateStatus::Ready(version)
            }
            Err(error) => {
                eprintln!("update: {error}");
                UpdateStatus::Failed
            }
        };
        drop(file);
        status
    }

    /// Verifies the bytes read through `file`, which are dropped before returning.
    fn verified_open(&self, file: &mut File, ready: &Ready) -> bool {
        let Some(signature) = fs::read(&ready.signature)
            .ok()
            .and_then(|bytes| parse_signature(&bytes))
        else {
            return false;
        };
        let mut installer = Vec::new();
        let name = self.kind.asset_name(ready.version);
        file.read_to_end(&mut installer).is_ok()
            && verify(
                &installer,
                &name,
                ready.version,
                &signature,
                &self.public_key,
            )
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Writes next to `path` and renames, so a cut download never looks complete.
fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    fs::write(&part, bytes)?;
    fs::rename(&part, path)
}

/// Deletes the downloads whose names start with `prefix` (packages, signatures and partial
/// files), except those of `keep`.
fn remove_installers(folder: &Path, prefix: &str, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let kept = keep.is_some_and(|keep| name == keep || name.strip_suffix(".sig") == Some(keep));
        if name.starts_with(prefix) && !kept {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{sha256_hex, signed_message, to_hex};
    use ed25519_dalek::{Signer, SigningKey};
    use std::cell::RefCell;

    const API: &str = "https://api.test/latest";
    const SETUP: &str = "https://downloads.test/setup";
    const SIG: &str = "https://downloads.test/sig";
    const SEED: [u8; 32] = [7; 32];

    struct Server {
        routes: Vec<(String, Vec<u8>)>,
        asked: RefCell<Vec<String>>,
    }

    impl Server {
        fn get(&self, url: &str, limit: u64) -> io::Result<Vec<u8>> {
            self.asked.borrow_mut().push(url.to_owned());
            let body = self
                .routes
                .iter()
                .find(|(route, _)| route == url)
                .map(|(_, body)| body.clone())
                .ok_or_else(|| io::Error::other("404"))?;
            if body.len() as u64 > limit {
                return Err(io::Error::other("too large"));
            }
            Ok(body)
        }
    }

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tex-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    fn signature(installer: &[u8], version: &str) -> Vec<u8> {
        let version = Version::parse(version).unwrap();
        signed(
            installer,
            &InstallKind::Windows.asset_name(version),
            version,
        )
    }

    fn signed(package: &[u8], asset: &str, version: Version) -> Vec<u8> {
        let message = signed_message(asset, version, &sha256_hex(package)).unwrap();
        let signature = SigningKey::from_bytes(&SEED).sign(&message).to_bytes();
        format!("{}\n", to_hex(&signature)).into_bytes()
    }

    fn release(version: &str, size: usize) -> Vec<u8> {
        format!(
            r#"{{"tag_name":"v{version}","assets":[{{"name":"Tex-Setup-{version}.exe","browser_download_url":"{SETUP}","size":{size}}},{{"name":"Tex-Setup-{version}.exe.sig","browser_download_url":"{SIG}","size":129}}]}}"#
        )
        .into_bytes()
    }

    fn server(version: &str, installer: &[u8], signature: Vec<u8>) -> Server {
        Server {
            routes: vec![
                (API.into(), release(version, installer.len())),
                (SETUP.into(), installer.to_vec()),
                (SIG.into(), signature),
            ],
            asked: RefCell::default(),
        }
    }

    fn checker<'a>(
        server: &'a Server,
        folder: &Path,
    ) -> Checker<impl Fn(&str, u64) -> io::Result<Vec<u8>> + 'a> {
        Checker {
            api: API.to_owned(),
            folder: folder.to_owned(),
            kind: InstallKind::Windows,
            current: Version::parse("0.4.2").unwrap(),
            public_key: SigningKey::from_bytes(&SEED).verifying_key().to_bytes(),
            fetch: move |url: &str, limit: u64| server.get(url, limit),
        }
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn up_to_date_when_the_latest_release_is_not_newer() {
        let folder = folder("current");
        let server = server("0.4.2", b"setup", signature(b"setup", "0.4.2"));
        assert_eq!(checker(&server, &folder).check(&|_| {}).unwrap(), None);
        assert_eq!(*server.asked.borrow(), [API]);
    }

    #[test]
    fn downloads_and_verifies_a_newer_release() {
        let folder = folder("download");
        let server = server("0.5.0", b"setup 0.5.0", signature(b"setup 0.5.0", "0.5.0"));
        let statuses = RefCell::new(Vec::new());
        let ready = checker(&server, &folder)
            .check(&|status| statuses.borrow_mut().push(status))
            .unwrap()
            .unwrap();
        assert_eq!(ready.version, Version::parse("0.5.0").unwrap());
        assert_eq!(ready.installer, folder.join("Tex-Setup-0.5.0.exe"));
        assert_eq!(ready.signature, folder.join("Tex-Setup-0.5.0.exe.sig"));
        assert_eq!(fs::read(&ready.installer).unwrap(), b"setup 0.5.0");
        assert_eq!(
            *statuses.borrow(),
            [UpdateStatus::Downloading("0.5.0".into())]
        );
        assert_eq!(
            names(&folder),
            ["Tex-Setup-0.5.0.exe", "Tex-Setup-0.5.0.exe.sig"]
        );
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn reuses_a_verified_download() {
        let folder = folder("reuse");
        let server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        let checker = checker(&server, &folder);
        checker.check(&|_| {}).unwrap();
        server.asked.borrow_mut().clear();
        let statuses = RefCell::new(Vec::new());
        let ready = checker
            .check(&|status| statuses.borrow_mut().push(status))
            .unwrap();
        assert!(ready.is_some());
        assert_eq!(*server.asked.borrow(), [API]);
        assert!(statuses.borrow().is_empty());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn refuses_an_installer_that_does_not_match_its_signature() {
        let folder = folder("forged");
        for signature in [
            signature(b"other", "0.5.0"),
            signature(b"setup", "0.4.9"),
            b"not hex".to_vec(),
        ] {
            let server = server("0.5.0", b"setup", signature);
            assert!(checker(&server, &folder).check(&|_| {}).is_err());
            assert!(!folder.join("Tex-Setup-0.5.0.exe").exists());
        }
    }

    #[test]
    fn refuses_an_installer_of_the_wrong_size() {
        let folder = folder("size");
        let mut server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        server.routes[0].1 = release("0.5.0", 4);
        assert!(checker(&server, &folder).check(&|_| {}).is_err());
        assert!(!folder.join("Tex-Setup-0.5.0.exe").exists());
    }

    #[test]
    fn refuses_an_installer_shorter_than_its_asset() {
        let folder = folder("short");
        let mut server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        server.routes[0].1 = release("0.5.0", 10);
        assert!(checker(&server, &folder).check(&|_| {}).is_err());
        assert!(!folder.join("Tex-Setup-0.5.0.exe").exists());
    }

    #[test]
    fn an_answer_that_is_not_a_release_is_an_error_and_keeps_downloads() {
        let folder = folder("not-release");
        let mut server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        checker(&server, &folder).check(&|_| {}).unwrap();
        let bad_tag = String::from_utf8(release("0.5.0", 5))
            .unwrap()
            .replace("\"v0.5.0\"", "\"latest\"");
        for answer in [b"not json".to_vec(), bad_tag.into_bytes()] {
            server.routes[0].1 = answer;
            assert!(checker(&server, &folder).check(&|_| {}).is_err());
            assert_eq!(
                names(&folder),
                ["Tex-Setup-0.5.0.exe", "Tex-Setup-0.5.0.exe.sig"]
            );
        }
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_newer_release_without_its_signature_has_nothing_to_install() {
        let folder = folder("no-sig");
        let mut server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        checker(&server, &folder).check(&|_| {}).unwrap();
        let no_signature = String::from_utf8(release("0.5.1", 5))
            .unwrap()
            .replace("Tex-Setup-0.5.1.exe.sig", "notes.txt");
        server.routes[0].1 = no_signature.into_bytes();
        assert_eq!(checker(&server, &folder).check(&|_| {}).unwrap(), None);
        assert_eq!(*server.asked.borrow().last().unwrap(), API);
        assert_eq!(
            names(&folder),
            ["Tex-Setup-0.5.0.exe", "Tex-Setup-0.5.0.exe.sig"]
        );
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn deletes_a_download_that_fails_verification_before_downloading_again() {
        let folder = folder("bad-download");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("Tex-Setup-0.5.0.exe"), b"evil").unwrap();
        fs::write(
            folder.join("Tex-Setup-0.5.0.exe.sig"),
            signature(b"setup", "0.5.0"),
        )
        .unwrap();
        let mut server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        server.routes.retain(|(route, _)| route != SETUP);
        assert!(checker(&server, &folder).check(&|_| {}).is_err());
        assert!(names(&folder).is_empty());
        fs::remove_dir_all(&folder).unwrap();
    }

    fn run_all(
        checker: &Checker<impl Fn(&str, u64) -> io::Result<Vec<u8>>>,
        commands: &[Command],
    ) -> Vec<UpdateStatus> {
        let (sender, receiver) = mpsc::channel();
        for command in commands {
            sender.send(*command).unwrap();
        }
        drop(sender);
        let statuses = RefCell::new(Vec::new());
        run(&receiver, checker, &|status| {
            statuses.borrow_mut().push(status)
        });
        statuses.into_inner()
    }

    #[test]
    fn apply_without_a_ready_installer_is_ignored() {
        let folder = folder("apply-idle");
        let server = server("0.4.2", b"setup", signature(b"setup", "0.4.2"));
        let statuses = run_all(
            &checker(&server, &folder),
            &[Command::Apply, Command::Check, Command::Apply],
        );
        assert_eq!(statuses, [UpdateStatus::Checking, UpdateStatus::UpToDate]);
    }

    #[test]
    fn a_failed_check_forgets_the_ready_installer() {
        let folder = folder("apply-failed");
        let server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        let answers = std::cell::Cell::new(0);
        let base = checker(&server, &folder);
        let checker = Checker {
            api: base.api,
            folder: base.folder,
            kind: base.kind,
            current: base.current,
            public_key: base.public_key,
            fetch: |url: &str, limit: u64| {
                if url == API {
                    answers.set(answers.get() + 1);
                    if answers.get() > 1 {
                        return Err(io::Error::other("offline"));
                    }
                }
                server.get(url, limit)
            },
        };
        let statuses = run_all(&checker, &[Command::Check, Command::Check, Command::Apply]);
        assert_eq!(
            statuses,
            [
                UpdateStatus::Checking,
                UpdateStatus::Downloading("0.5.0".into()),
                UpdateStatus::Ready("0.5.0".into()),
                UpdateStatus::Checking,
                UpdateStatus::Failed,
            ]
        );
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_request_is_an_error() {
        let folder = folder("offline");
        let server = Server {
            routes: Vec::new(),
            asked: RefCell::default(),
        };
        assert!(checker(&server, &folder).check(&|_| {}).is_err());
    }

    #[test]
    fn keeps_only_the_installer_of_the_newest_release() {
        let folder = folder("cleanup");
        fs::create_dir_all(&folder).unwrap();
        for name in [
            "Tex-Setup-0.4.9.exe",
            "Tex-Setup-0.4.9.exe.sig",
            "Tex-Setup-0.5.0.exe.part",
            "notes.txt",
        ] {
            fs::write(folder.join(name), b"x").unwrap();
        }
        let server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        checker(&server, &folder).check(&|_| {}).unwrap();
        assert_eq!(
            names(&folder),
            [
                "Tex-Setup-0.5.0.exe",
                "Tex-Setup-0.5.0.exe.sig",
                "notes.txt"
            ]
        );
        let current = self::server("0.4.2", b"setup", signature(b"setup", "0.4.2"));
        checker(&current, &folder).check(&|_| {}).unwrap();
        assert_eq!(names(&folder), ["notes.txt"]);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn install_refuses_a_tampered_installer() {
        let folder = folder("tampered");
        let server = server("0.5.0", b"setup", signature(b"setup", "0.5.0"));
        let checker = checker(&server, &folder);
        let ready = checker.check(&|_| {}).unwrap().unwrap();
        fs::write(&ready.installer, b"evil").unwrap();
        assert_eq!(checker.install(&ready, &|_| {}), UpdateStatus::Failed);
        assert!(!ready.installer.exists() && !ready.signature.exists());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn statuses_name_their_state_and_version() {
        assert_eq!(UpdateStatus::Idle.parts(), (0, ""));
        assert_eq!(UpdateStatus::Checking.parts(), (1, ""));
        assert_eq!(UpdateStatus::UpToDate.parts(), (2, ""));
        assert_eq!(
            UpdateStatus::Downloading("0.5.0".into()).parts(),
            (3, "0.5.0")
        );
        assert_eq!(UpdateStatus::Ready("0.5.0".into()).parts(), (4, "0.5.0"));
        assert_eq!(UpdateStatus::Failed.parts(), (5, ""));
        assert_eq!(
            UpdateStatus::Installing("0.5.0".into()).parts(),
            (6, "0.5.0")
        );
        assert_eq!(
            UpdateStatus::Ready("0.5.0".into()).ready_version(),
            Some("0.5.0")
        );
        assert_eq!(
            UpdateStatus::Installing("0.5.0".into()).ready_version(),
            None
        );
        assert!(UpdateStatus::Checking.busy());
        assert!(UpdateStatus::Downloading("0.5.0".into()).busy());
        assert!(UpdateStatus::Installing("0.5.0".into()).busy());
        assert!(!UpdateStatus::Ready("0.5.0".into()).busy());
        assert!(!UpdateStatus::Failed.busy());
    }

    #[test]
    fn a_check_goes_stale_after_a_day() {
        let now = Instant::now();
        assert!(is_stale(None, now));
        assert!(!is_stale(Some(now), now + Duration::from_secs(60)));
        assert!(is_stale(Some(now), now + Duration::from_secs(24 * 60 * 60)));
    }

    #[test]
    fn checks_the_latest_release_of_the_repository() {
        assert_eq!(
            latest_url("LuizFer1/Tex"),
            "https://api.github.com/repos/LuizFer1/Tex/releases/latest"
        );
    }

    const DEB: &str = "https://downloads.test/deb";
    const DEB_SIG: &str = "https://downloads.test/deb.sig";

    fn deb_server(signature: Vec<u8>) -> Server {
        let answer = format!(
            r#"{{"tag_name":"v0.5.0","assets":[{{"name":"Tex-Setup-0.5.0.exe","browser_download_url":"{SETUP}","size":5}},{{"name":"tex_0.5.0_amd64.deb","browser_download_url":"{DEB}","size":7}},{{"name":"tex_0.5.0_amd64.deb.sig","browser_download_url":"{DEB_SIG}","size":129}}]}}"#
        );
        Server {
            routes: vec![
                (API.into(), answer.into_bytes()),
                (DEB.into(), b"package".to_vec()),
                (DEB_SIG.into(), signature),
            ],
            asked: RefCell::default(),
        }
    }

    fn deb_checker<'a>(
        server: &'a Server,
        folder: &Path,
    ) -> Checker<impl Fn(&str, u64) -> io::Result<Vec<u8>> + 'a> {
        Checker {
            kind: InstallKind::Deb,
            ..checker(server, folder)
        }
    }

    #[test]
    fn downloads_the_package_of_its_install_kind() {
        let folder = folder("deb");
        let version = Version::parse("0.5.0").unwrap();
        let server = deb_server(signed(b"package", "tex_0.5.0_amd64.deb", version));
        let ready = deb_checker(&server, &folder)
            .check(&|_| {})
            .unwrap()
            .unwrap();
        assert_eq!(ready.installer, folder.join("tex_0.5.0_amd64.deb"));
        assert_eq!(
            names(&folder),
            ["tex_0.5.0_amd64.deb", "tex_0.5.0_amd64.deb.sig"]
        );
        assert!(!server.asked.borrow().iter().any(|url| url == SETUP));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn refuses_a_package_signed_for_another_asset() {
        let folder = folder("deb-forged");
        let version = Version::parse("0.5.0").unwrap();
        for forged in [
            signed(b"package", "Tex-Setup-0.5.0.exe", version),
            signed(b"package", "tex-0.5.0.x86_64.rpm", version),
        ] {
            let server = deb_server(forged);
            assert!(deb_checker(&server, &folder).check(&|_| {}).is_err());
            assert!(!folder.join("tex_0.5.0_amd64.deb").exists());
        }
    }

    #[test]
    fn without_an_install_kind_every_check_is_up_to_date() {
        let (sender, receiver) = mpsc::channel();
        for command in [Command::Check, Command::Apply, Command::Check] {
            sender.send(command).unwrap();
        }
        drop(sender);
        let statuses = RefCell::new(Vec::new());
        run_without_updates(&receiver, &|status| statuses.borrow_mut().push(status));
        assert_eq!(
            statuses.into_inner(),
            [UpdateStatus::UpToDate, UpdateStatus::UpToDate]
        );
    }
}
