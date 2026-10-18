use std::fmt;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const PUBLIC_KEY: [u8; 32] = *include_bytes!("../update.pub");
const REPOSITORY: &str = "LuizFer1/Tex";

/// Exit code of `tex --apply-update` for a package that is not a signed Tex package newer
/// than the installed one.
#[cfg(not(windows))]
pub const REFUSED_EXIT: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `major.minor.patch` with an optional leading `v`; anything else is `None`.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let version = Version {
            major: number(parts.next()?)?,
            minor: number(parts.next()?)?,
            patch: number(parts.next()?)?,
        };
        parts.next().is_none().then_some(version)
    }

    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION"))
            .expect("the workspace version is major.minor.patch")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// How this copy of Tex was installed, which names the release asset it updates from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Windows,
    Deb,
    Rpm,
    Tar,
}

impl InstallKind {
    pub const ALL: [InstallKind; 4] = [
        InstallKind::Windows,
        InstallKind::Deb,
        InstallKind::Rpm,
        InstallKind::Tar,
    ];

    pub fn asset_name(self, version: Version) -> String {
        match self {
            InstallKind::Windows => format!("Tex-Setup-{version}.exe"),
            InstallKind::Deb => format!("tex_{version}_amd64.deb"),
            InstallKind::Rpm => format!("tex-{version}.x86_64.rpm"),
            InstallKind::Tar => format!("tex-{version}-linux-x86_64.tar.gz"),
        }
    }

    /// The kind whose asset for `version` is named `name`.
    pub fn for_asset(name: &str, version: Version) -> Option<InstallKind> {
        InstallKind::ALL
            .into_iter()
            .find(|kind| kind.asset_name(version) == name)
    }

    /// How the names of this kind's downloads start, to clean up old ones.
    pub fn prefix(self) -> &'static str {
        match self {
            InstallKind::Windows => "Tex-Setup-",
            InstallKind::Deb => "tex_",
            InstallKind::Rpm | InstallKind::Tar => "tex-",
        }
    }

    #[cfg(windows)]
    pub fn current() -> Option<InstallKind> {
        Some(InstallKind::Windows)
    }

    /// The `install-kind` file next to the executable, which each package writes. Debug
    /// builds read `TEX_INSTALL_KIND` first. `None` for a build that no package installed.
    #[cfg(not(windows))]
    pub fn current() -> Option<InstallKind> {
        if cfg!(debug_assertions)
            && let Ok(kind) = std::env::var("TEX_INSTALL_KIND")
        {
            return InstallKind::parse(&kind);
        }
        let exe = std::env::current_exe().ok()?;
        let text = std::fs::read_to_string(exe.parent()?.join("install-kind")).ok()?;
        InstallKind::parse(&text)
    }

    #[cfg(not(windows))]
    pub fn parse(text: &str) -> Option<InstallKind> {
        match text.trim() {
            "deb" => Some(InstallKind::Deb),
            "rpm" => Some(InstallKind::Rpm),
            "tar" => Some(InstallKind::Tar),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub installer: Asset,
    pub signature: Asset,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

impl Release {
    /// The release in a GitHub `releases/latest` answer, when it has both the package of
    /// `kind` and its signature.
    pub fn from_json(bytes: &[u8], kind: InstallKind) -> Option<Release> {
        let raw: RawRelease = serde_json::from_slice(bytes).ok()?;
        let version = Version::parse(&raw.tag_name)?;
        let installer = kind.asset_name(version);
        let asset = |name: &str| {
            raw.assets
                .iter()
                .find(|asset| asset.name == name)
                .map(|asset| Asset {
                    url: asset.browser_download_url.clone(),
                    size: asset.size,
                })
        };
        Some(Release {
            version,
            installer: asset(&installer)?,
            signature: asset(&format!("{installer}.sig"))?,
        })
    }

    /// The version a GitHub `releases/latest` answer names, whatever its assets; `None` when
    /// the answer is not a release.
    pub fn version_in(bytes: &[u8]) -> Option<Version> {
        let raw: RawRelease = serde_json::from_slice(bytes).ok()?;
        Version::parse(&raw.tag_name)
    }
}

/// `dono/repo` whose releases are checked. Debug builds read `TEX_UPDATE_REPO` first.
pub fn repository() -> String {
    if cfg!(debug_assertions)
        && let Ok(repository) = std::env::var("TEX_UPDATE_REPO")
        && !repository.is_empty()
    {
        return repository;
    }
    REPOSITORY.to_owned()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(Sha256::digest(bytes).as_slice())
}

/// What a release signs for `asset`: the Windows installer keeps the `tex-setup` message that
/// Tex 0.4.2 verifies; the Linux packages name the asset, so one cannot pass for another.
/// `None` for a name that is no asset of `version`.
pub fn signed_message(asset: &str, version: Version, sha256: &str) -> Option<Vec<u8>> {
    let mut message = match InstallKind::for_asset(asset, version)? {
        InstallKind::Windows => b"tex-setup\0".to_vec(),
        _ => {
            let mut message = b"tex-package\0".to_vec();
            message.extend_from_slice(asset.as_bytes());
            message.push(0);
            message
        }
    };
    message.extend_from_slice(version.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(sha256.as_bytes());
    Some(message)
}

pub fn verify(
    package: &[u8],
    asset: &str,
    version: Version,
    signature: &[u8; 64],
    public_key: &[u8; 32],
) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Some(message) = signed_message(asset, version, &sha256_hex(package)) else {
        return false;
    };
    key.verify_strict(&message, &Signature::from_bytes(signature))
        .is_ok()
}

/// The 64 signature bytes of a `.sig` file: hex on one line.
pub fn parse_signature(bytes: &[u8]) -> Option<[u8; 64]> {
    from_hex(std::str::from_utf8(bytes).ok()?)?.try_into().ok()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const SEED: [u8; 32] = [7; 32];
    const EXE: &str = "Tex-Setup-0.4.3.exe";
    const DEB: &str = "tex_0.4.3_amd64.deb";

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn public_key(seed: &[u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(seed).verifying_key().to_bytes()
    }

    fn sign(package: &[u8], asset: &str, version: &str) -> [u8; 64] {
        let message = signed_message(asset, v(version), &sha256_hex(package)).unwrap();
        SigningKey::from_bytes(&SEED).sign(&message).to_bytes()
    }

    const LATEST: &str = r#"{
        "url": "https://api.github.com/repos/LuizFer1/Tex/releases/1",
        "html_url": "https://github.com/LuizFer1/Tex/releases/tag/v0.4.3",
        "id": 1,
        "tag_name": "v0.4.3",
        "name": "Tex 0.4.3",
        "draft": false,
        "prerelease": false,
        "published_at": "2026-10-20T12:00:00Z",
        "assets": [
            {
                "name": "Tex-Setup-0.4.3.exe.sig",
                "size": 129,
                "browser_download_url": "https://github.com/LuizFer1/Tex/releases/download/v0.4.3/Tex-Setup-0.4.3.exe.sig",
                "uploader": { "login": "LuizFer1" }
            },
            {
                "name": "Tex-Setup-0.4.3.exe",
                "size": 9876543,
                "browser_download_url": "https://github.com/LuizFer1/Tex/releases/download/v0.4.3/Tex-Setup-0.4.3.exe",
                "uploader": { "login": "LuizFer1" }
            }
        ],
        "body": "notes"
    }"#;

    const LINUX: &str = r#"{
        "tag_name": "v0.5.0",
        "assets": [
            { "name": "tex_0.5.0_amd64.deb", "size": 10, "browser_download_url": "https://downloads.test/deb" },
            { "name": "tex_0.5.0_amd64.deb.sig", "size": 129, "browser_download_url": "https://downloads.test/deb.sig" },
            { "name": "tex-0.5.0.x86_64.rpm", "size": 11, "browser_download_url": "https://downloads.test/rpm" },
            { "name": "tex-0.5.0.x86_64.rpm.sig", "size": 129, "browser_download_url": "https://downloads.test/rpm.sig" },
            { "name": "tex-0.5.0-linux-x86_64.tar.gz", "size": 12, "browser_download_url": "https://downloads.test/tar" },
            { "name": "tex-0.5.0-linux-x86_64.tar.gz.sig", "size": 129, "browser_download_url": "https://downloads.test/tar.sig" }
        ]
    }"#;

    #[test]
    fn reads_versions_with_an_optional_v() {
        assert_eq!(
            v("0.4.2"),
            Version {
                major: 0,
                minor: 4,
                patch: 2
            }
        );
        assert_eq!(v("v10.0.1").to_string(), "10.0.1");
        for bad in [
            "", "v", "0.4", "0.4.2.1", "0.4.x", "0.4.-1", "+0.4.2", "0..2", " 0.4.2",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn compares_versions_by_number() {
        assert!(v("0.4.10") > v("0.4.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.5.0") > v("0.4.9"));
        assert_eq!(v("v0.4.2"), v("0.4.2"));
    }

    #[test]
    fn reads_the_latest_release_and_its_assets() {
        let release = Release::from_json(LATEST.as_bytes(), InstallKind::Windows).unwrap();
        assert_eq!(release.version, v("0.4.3"));
        assert_eq!(
            release.installer,
            Asset {
                url: "https://github.com/LuizFer1/Tex/releases/download/v0.4.3/Tex-Setup-0.4.3.exe"
                    .into(),
                size: 9_876_543
            }
        );
        assert!(release.signature.url.ends_with("Tex-Setup-0.4.3.exe.sig"));
    }

    #[test]
    fn ignores_releases_without_both_assets() {
        let windows = InstallKind::Windows;
        assert_eq!(
            Release::from_json(br#"{"tag_name":"v0.4.3"}"#, windows),
            None
        );
        let no_signature = LATEST.replace("Tex-Setup-0.4.3.exe.sig", "notes.txt");
        assert_eq!(Release::from_json(no_signature.as_bytes(), windows), None);
        let other_version = LATEST.replace("\"v0.4.3\"", "\"v0.4.4\"");
        assert_eq!(Release::from_json(other_version.as_bytes(), windows), None);
        let bad_tag = LATEST.replace("\"v0.4.3\"", "\"latest\"");
        assert_eq!(Release::from_json(bad_tag.as_bytes(), windows), None);
        assert_eq!(Release::from_json(b"not json", windows), None);
    }

    #[test]
    fn picks_the_package_of_the_install_kind() {
        for (kind, url, size) in [
            (InstallKind::Deb, "https://downloads.test/deb", 10),
            (InstallKind::Rpm, "https://downloads.test/rpm", 11),
            (InstallKind::Tar, "https://downloads.test/tar", 12),
        ] {
            let release = Release::from_json(LINUX.as_bytes(), kind).unwrap();
            assert_eq!(
                release.installer,
                Asset {
                    url: url.into(),
                    size
                }
            );
            assert_eq!(release.signature.url, format!("{url}.sig"));
        }
        assert_eq!(
            Release::from_json(LINUX.as_bytes(), InstallKind::Windows),
            None
        );
    }

    #[test]
    fn names_the_asset_of_each_kind() {
        let version = v("0.5.0");
        assert_eq!(
            InstallKind::Windows.asset_name(version),
            "Tex-Setup-0.5.0.exe"
        );
        assert_eq!(InstallKind::Deb.asset_name(version), "tex_0.5.0_amd64.deb");
        assert_eq!(InstallKind::Rpm.asset_name(version), "tex-0.5.0.x86_64.rpm");
        assert_eq!(
            InstallKind::Tar.asset_name(version),
            "tex-0.5.0-linux-x86_64.tar.gz"
        );
        for kind in InstallKind::ALL {
            let name = kind.asset_name(version);
            assert_eq!(InstallKind::for_asset(&name, version), Some(kind));
            assert!(name.starts_with(kind.prefix()), "{name}");
        }
        assert_eq!(
            InstallKind::for_asset("tex_0.5.0_amd64.deb", v("0.5.1")),
            None
        );
        assert_eq!(InstallKind::for_asset("notes.txt", version), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn reads_the_install_kind_file() {
        assert_eq!(InstallKind::parse("deb\n"), Some(InstallKind::Deb));
        assert_eq!(InstallKind::parse(" rpm "), Some(InstallKind::Rpm));
        assert_eq!(InstallKind::parse("tar\r\n"), Some(InstallKind::Tar));
        assert_eq!(InstallKind::parse("exe"), None);
        assert_eq!(InstallKind::parse(""), None);
    }

    #[test]
    fn reads_the_version_of_any_release() {
        assert_eq!(Release::version_in(LATEST.as_bytes()), Some(v("0.4.3")));
        assert_eq!(
            Release::version_in(br#"{"tag_name":"v0.4.4"}"#),
            Some(v("0.4.4"))
        );
        let bad_tag = LATEST.replace("\"v0.4.3\"", "\"latest\"");
        assert_eq!(Release::version_in(bad_tag.as_bytes()), None);
        assert_eq!(Release::version_in(b"not json"), None);
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(to_hex(&[0, 15, 255]), "000fff");
        assert_eq!(from_hex(" 000fff\n"), Some(vec![0, 15, 255]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("+f"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn the_signed_message_names_the_version_and_the_hash() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            signed_message(EXE, v("0.4.3"), "ab12"),
            Some(b"tex-setup\x000.4.3\x00ab12".to_vec())
        );
        assert_eq!(
            signed_message(DEB, v("0.4.3"), "ab12"),
            Some(b"tex-package\x00tex_0.4.3_amd64.deb\x000.4.3\x00ab12".to_vec())
        );
        assert_eq!(signed_message(EXE, v("0.4.4"), "ab12"), None);
        assert_eq!(signed_message("notes.txt", v("0.4.3"), "ab12"), None);
    }

    #[test]
    fn verifies_signatures() {
        let key = public_key(&SEED);
        let signature = sign(b"setup", EXE, "0.4.3");
        assert!(verify(b"setup", EXE, v("0.4.3"), &signature, &key));
        assert!(!verify(b"setup!", EXE, v("0.4.3"), &signature, &key));
        assert!(!verify(
            b"setup",
            "Tex-Setup-0.4.4.exe",
            v("0.4.4"),
            &signature,
            &key
        ));
        assert!(!verify(
            b"setup",
            EXE,
            v("0.4.3"),
            &signature,
            &public_key(&[8; 32])
        ));
        assert!(!verify(b"setup", EXE, v("0.4.3"), &[0; 64], &key));
    }

    #[test]
    fn a_package_signature_names_its_asset() {
        let key = public_key(&SEED);
        let signature = sign(b"package", DEB, "0.4.3");
        assert!(verify(b"package", DEB, v("0.4.3"), &signature, &key));
        for other in [
            "tex-0.4.3.x86_64.rpm",
            "tex-0.4.3-linux-x86_64.tar.gz",
            "Tex-Setup-0.4.3.exe",
        ] {
            assert!(
                !verify(b"package", other, v("0.4.3"), &signature, &key),
                "{other}"
            );
        }
        assert!(!verify(
            b"package",
            "tex_0.4.4_amd64.deb",
            v("0.4.4"),
            &signature,
            &key
        ));
    }

    #[test]
    fn reads_signature_files() {
        let signature = sign(b"setup", EXE, "0.4.3");
        let file = format!("{}\r\n", to_hex(&signature));
        assert_eq!(parse_signature(file.as_bytes()), Some(signature));
        assert_eq!(parse_signature(b"abcd"), None);
        assert_eq!(parse_signature(&[0xff, 0xfe]), None);
    }
}
