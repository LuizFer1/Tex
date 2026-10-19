use std::fs;
use std::path::{Path, PathBuf};

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn linux(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../installer/linux")
        .join(name)
}

fn text(name: &str) -> String {
    fs::read_to_string(linux(name))
        .unwrap_or_else(|error| panic!("{name}: {error}"))
        .replace("\r\n", "\n")
}

fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", path.display());
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (number(16), number(20))
}

#[test]
fn the_launchers_run_the_installed_app() {
    let desktop = text("io.github.LuizFer1.Tex.desktop");
    assert!(desktop.starts_with("[Desktop Entry]\n"));
    assert!(desktop.contains("\nType=Application\n"));
    assert!(desktop.contains("\nExec=/usr/lib/tex/tex\n"));
    assert!(desktop.contains("\nIcon=io.github.LuizFer1.Tex\n"));
    let autostart = text("io.github.LuizFer1.Tex.autostart.desktop");
    assert!(autostart.starts_with("[Desktop Entry]\n"));
    assert!(autostart.contains("\nExec=/usr/lib/tex/tex --autostart\n"));
    assert!(autostart.contains("\nNoDisplay=true\n"));
}

#[test]
fn the_update_policy_names_the_installed_app() {
    let policy = text("io.github.LuizFer1.Tex.update.policy");
    assert!(policy.contains(r#"<action id="io.github.LuizFer1.Tex.update">"#));
    assert!(policy.contains(
        r#"<annotate key="org.freedesktop.policykit.exec.path">/usr/lib/tex/tex</annotate>"#
    ));
    assert!(policy.contains("<allow_active>auth_admin_keep</allow_active>"));
    assert!(policy.contains("Instalar a atualização do Tex"));
}

#[test]
fn metainfo_names_the_launcher() {
    let metainfo = text("io.github.LuizFer1.Tex.metainfo.xml");
    assert!(metainfo.contains("<id>io.github.LuizFer1.Tex</id>"));
    assert!(
        metainfo.contains(
            r#"<launchable type="desktop-id">io.github.LuizFer1.Tex.desktop</launchable>"#
        )
    );
    assert!(metainfo.contains("<project_license>PolyForm-Noncommercial-1.0.0</project_license>"));
}

#[test]
fn each_package_names_its_install_kind() {
    for kind in ["deb", "rpm", "tar"] {
        assert_eq!(text(&format!("install-kind/{kind}")), format!("{kind}\n"));
    }
}

#[test]
fn icons_come_in_every_size() {
    for size in SIZES {
        assert_eq!(png_size(&linux(&format!("icons/{size}.png"))), (size, size));
    }
}
