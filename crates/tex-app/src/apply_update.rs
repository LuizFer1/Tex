use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::update::{self, InstallKind, Version, parse_signature, verify};

const WORK: &str = "/var/lib/tex/updates";
const PACKAGE_LIMIT: u64 = 200 * 1024 * 1024;
const SIGNATURE_LIMIT: u64 = 1024;
const USAGE_EXIT: i32 = 2;
const FAILED_EXIT: i32 = 4;
const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// `tex --apply-update <package> <version>`, which the app runs through `pkexec`.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub package: PathBuf,
    pub version: Version,
}

struct Package {
    kind: InstallKind,
    name: String,
    version: Version,
    bytes: Vec<u8>,
}

/// Runs `tex --apply-update` and exits when the command line asks for it. Any other use as
/// root also exits: after the password, `pkexec` may run Tex again without asking for a while.
pub fn exit_if_asked() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let root = unsafe { libc::geteuid() } == 0;
    match parse_args(&args) {
        Some(Ok(request)) if root => process::exit(apply(&request)),
        Some(Ok(_)) => {
            eprintln!("tex --apply-update must run as root, through pkexec");
            process::exit(USAGE_EXIT);
        }
        Some(Err(usage)) => {
            eprintln!("{usage}");
            process::exit(USAGE_EXIT);
        }
        None if root => {
            eprintln!("tex: do not run Tex as root");
            process::exit(USAGE_EXIT);
        }
        None => {}
    }
}

/// The request when the first argument is `--apply-update`; `Err` with the usage when the
/// rest does not fit.
fn parse_args(args: &[OsString]) -> Option<Result<Request, String>> {
    if args.first()? != "--apply-update" {
        return None;
    }
    let usage = || "usage: tex --apply-update <package> <version>".to_owned();
    let [_, package, version] = args else {
        return Some(Err(usage()));
    };
    let Some(version) = version.to_str().and_then(Version::parse) else {
        return Some(Err(usage()));
    };
    Some(Ok(Request {
        package: PathBuf::from(package),
        version,
    }))
}

/// Verifies and installs: 0 when installed, [`update::REFUSED_EXIT`] for a package that is
/// not a signed Tex package newer than this one, [`FAILED_EXIT`] when installing fails.
fn apply(request: &Request) -> i32 {
    let package = match verified_package(request) {
        Ok(package) => package,
        Err(message) => {
            eprintln!("tex: {message}");
            return update::REFUSED_EXIT;
        }
    };
    match install(&package) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("tex: installing the update failed: {error}");
            FAILED_EXIT
        }
    }
}

/// The kind of a package Tex may install: a Linux asset named for `version`, newer than
/// `current`.
fn accepted(name: &str, version: Version, current: Version) -> Result<InstallKind, String> {
    if version <= current {
        return Err(format!(
            "{version} is not newer than the installed {current}"
        ));
    }
    match InstallKind::for_asset(name, version) {
        Some(InstallKind::Windows) | None => {
            Err(format!("{name} is not a Linux package of Tex {version}"))
        }
        Some(kind) => Ok(kind),
    }
}

/// Reads the package and its `.sig` and checks both against the key built into this program.
fn verified_package(request: &Request) -> Result<Package, String> {
    let name = request
        .package
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the package has no file name")?;
    let kind = accepted(name, request.version, Version::current())?;
    let mut signature_path = request.package.clone().into_os_string();
    signature_path.push(".sig");
    let signature = read_limited(Path::new(&signature_path), SIGNATURE_LIMIT)
        .ok()
        .and_then(|bytes| parse_signature(&bytes))
        .ok_or("the signature file is missing or not a signature")?;
    let bytes = read_limited(&request.package, PACKAGE_LIMIT)
        .map_err(|error| format!("{}: {error}", request.package.display()))?;
    if !verify(
        &bytes,
        name,
        request.version,
        &signature,
        &update::PUBLIC_KEY,
    ) {
        return Err(format!("{name} does not match its signature"));
    }
    Ok(Package {
        kind,
        name: name.to_owned(),
        version: request.version,
        bytes,
    })
}

/// Reads a regular file of at most `limit` bytes without following a final symbolic link.
fn read_limited(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file of the expected size",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(bytes)
}

/// Writes the verified bytes to a folder only root can open and installs that copy, so no
/// user process can swap the file between the check and the install.
fn install(package: &Package) -> io::Result<()> {
    let work = Path::new(WORK);
    let _ = fs::remove_dir_all(work);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(work)?;
    fs::set_permissions(work, fs::Permissions::from_mode(0o700))?;
    let copy = work.join(&package.name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&copy)?;
    file.write_all(&package.bytes)?;
    file.sync_all()?;
    drop(file);
    let unpacked = work.join("unpacked");
    fs::DirBuilder::new().mode(0o700).create(&unpacked)?;
    let prefix = tar_prefix(tex_platform::current_executable().as_deref());
    for step in steps(package.kind, &copy, &unpacked, package.version, &prefix) {
        run_step(&step)?;
    }
    let _ = fs::remove_dir_all(work);
    Ok(())
}

/// The commands that install `package`, root's copy of a package of `kind`.
fn steps(
    kind: InstallKind,
    package: &Path,
    unpacked: &Path,
    version: Version,
    prefix: &Path,
) -> Vec<Vec<OsString>> {
    let text = |part: &str| OsString::from(part);
    match kind {
        InstallKind::Deb => vec![vec![text("dpkg"), text("-i"), package.into()]],
        InstallKind::Rpm => vec![vec![text("rpm"), text("-U"), package.into()]],
        InstallKind::Tar => vec![
            vec![
                text("tar"),
                text("-xzf"),
                package.into(),
                text("-C"),
                unpacked.into(),
            ],
            vec![
                text("sh"),
                unpacked
                    .join(format!("tex-{version}-linux-x86_64"))
                    .join("install.sh")
                    .into(),
                text("--prefix"),
                prefix.into(),
            ],
        ],
        InstallKind::Windows => Vec::new(),
    }
}

/// Where a `.tar.gz` install went, from this program's path (`<prefix>/lib/tex/tex`);
/// `/usr/local` when it is somewhere else.
fn tar_prefix(exe: Option<&Path>) -> PathBuf {
    let installed = |exe: &Path| -> Option<PathBuf> {
        let tex = exe.parent()?;
        let lib = tex.parent()?;
        if tex.file_name()? != "tex" || lib.file_name()? != "lib" {
            return None;
        }
        lib.parent().map(Path::to_path_buf)
    };
    exe.and_then(installed)
        .unwrap_or_else(|| PathBuf::from("/usr/local"))
}

fn run_step(step: &[OsString]) -> io::Result<()> {
    let Some((program, args)) = step.split_first() else {
        return Ok(());
    };
    let status = Command::new(program)
        .args(args)
        .env("PATH", SAFE_PATH)
        .env_remove("DESTDIR")
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} exited with {status}",
            program.to_string_lossy()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn args(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tex-apply-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn text(steps: Vec<Vec<OsString>>) -> Vec<Vec<String>> {
        steps
            .into_iter()
            .map(|step| {
                step.into_iter()
                    .map(|part| part.into_string().unwrap())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn reads_the_apply_update_command_line() {
        assert_eq!(parse_args(&args(&[])), None);
        assert_eq!(parse_args(&args(&["--toggle"])), None);
        assert_eq!(
            parse_args(&args(&[
                "--apply-update",
                "/tmp/tex_0.5.0_amd64.deb",
                "0.5.0"
            ])),
            Some(Ok(Request {
                package: PathBuf::from("/tmp/tex_0.5.0_amd64.deb"),
                version: v("0.5.0")
            }))
        );
        for bad in [
            args(&["--apply-update"]),
            args(&["--apply-update", "/tmp/p"]),
            args(&["--apply-update", "/tmp/p", "latest"]),
            args(&["--apply-update", "/tmp/p", "0.5.0", "extra"]),
        ] {
            assert!(matches!(parse_args(&bad), Some(Err(_))), "{bad:?}");
        }
    }

    #[test]
    fn accepts_only_newer_linux_packages() {
        let current = v("0.4.2");
        let newer = v("0.5.0");
        assert_eq!(
            accepted("tex_0.5.0_amd64.deb", newer, current),
            Ok(InstallKind::Deb)
        );
        assert_eq!(
            accepted("tex-0.5.0.x86_64.rpm", newer, current),
            Ok(InstallKind::Rpm)
        );
        assert_eq!(
            accepted("tex-0.5.0-linux-x86_64.tar.gz", newer, current),
            Ok(InstallKind::Tar)
        );
        assert!(accepted("Tex-Setup-0.5.0.exe", newer, current).is_err());
        assert!(accepted("tex_0.4.9_amd64.deb", newer, current).is_err());
        assert!(accepted("evil.deb", newer, current).is_err());
        assert!(accepted("tex_0.4.2_amd64.deb", current, current).is_err());
        assert!(accepted("tex_0.4.1_amd64.deb", v("0.4.1"), current).is_err());
    }

    #[test]
    fn installs_each_kind_with_its_tool() {
        let package = Path::new("/var/lib/tex/updates/p");
        let unpacked = Path::new("/var/lib/tex/updates/unpacked");
        let prefix = Path::new("/usr/local");
        let run = |kind: InstallKind| text(steps(kind, package, unpacked, v("0.5.0"), prefix));
        assert_eq!(
            run(InstallKind::Deb),
            vec![vec!["dpkg", "-i", "/var/lib/tex/updates/p"]]
        );
        assert_eq!(
            run(InstallKind::Rpm),
            vec![vec!["rpm", "-U", "/var/lib/tex/updates/p"]]
        );
        assert_eq!(
            run(InstallKind::Tar),
            vec![
                vec![
                    "tar",
                    "-xzf",
                    "/var/lib/tex/updates/p",
                    "-C",
                    "/var/lib/tex/updates/unpacked"
                ],
                vec![
                    "sh",
                    "/var/lib/tex/updates/unpacked/tex-0.5.0-linux-x86_64/install.sh",
                    "--prefix",
                    "/usr/local"
                ],
            ]
        );
        assert!(run(InstallKind::Windows).is_empty());
    }

    #[test]
    fn a_tarball_updates_the_prefix_it_was_installed_in() {
        assert_eq!(
            tar_prefix(Some(Path::new("/usr/local/lib/tex/tex"))),
            PathBuf::from("/usr/local")
        );
        assert_eq!(
            tar_prefix(Some(Path::new("/opt/apps/lib/tex/tex"))),
            PathBuf::from("/opt/apps")
        );
        assert_eq!(
            tar_prefix(Some(Path::new("/home/ana/tex-target/debug/tex"))),
            PathBuf::from("/usr/local")
        );
        assert_eq!(tar_prefix(None), PathBuf::from("/usr/local"));
    }

    #[test]
    fn reads_only_small_regular_files() {
        let folder = temp("read");
        let file = folder.join("file");
        fs::write(&file, b"12345").unwrap();
        assert_eq!(read_limited(&file, 5).unwrap(), b"12345");
        assert!(read_limited(&file, 4).is_err());
        std::os::unix::fs::symlink(&file, folder.join("link")).unwrap();
        assert!(read_limited(&folder.join("link"), 5).is_err());
        assert!(read_limited(&folder, 5).is_err());
        assert!(read_limited(&folder.join("missing"), 5).is_err());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn refuses_a_package_without_a_valid_signature() {
        let folder = temp("unsigned");
        let package = folder.join("tex_99.0.0_amd64.deb");
        fs::write(&package, b"package").unwrap();
        let request = Request {
            package: package.clone(),
            version: v("99.0.0"),
        };
        let error = verified_package(&request).err().unwrap();
        assert!(error.contains("signature"), "{error}");
        fs::write(
            folder.join("tex_99.0.0_amd64.deb.sig"),
            format!("{}\n", "00".repeat(64)),
        )
        .unwrap();
        let error = verified_package(&request).err().unwrap();
        assert!(error.contains("does not match its signature"), "{error}");
        let wrong_name = Request {
            package: folder.join("other.deb"),
            version: v("99.0.0"),
        };
        assert!(verified_package(&wrong_name).is_err());
        fs::remove_dir_all(&folder).unwrap();
    }
}
