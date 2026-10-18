/// The systemd unit that runs `exe` as the search service. The packages install the same text
/// for `/usr/lib/tex/tex-indexer` (`installer/linux/tex-indexer.service`).
pub fn unit_text(exe: &str) -> String {
    let exe = if exe.contains(' ') {
        format!("\"{exe}\"")
    } else {
        exe.to_owned()
    };
    format!(
        "[Unit]
Description=Tex search indexer
After=local-fs.target

[Service]
Type=simple
ExecStart={exe} service
RuntimeDirectory=tex
RuntimeDirectoryMode=0755
StateDirectory=tex
StateDirectoryMode=0700
Restart=on-failure
Nice=10
IOSchedulingClass=idle

[Install]
WantedBy=multi-user.target
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_the_service_subcommand() {
        let unit = unit_text("/home/ana/tex-target/debug/tex-indexer");
        assert!(unit.contains("\nExecStart=/home/ana/tex-target/debug/tex-indexer service\n"));
        assert!(unit.contains("\nRuntimeDirectory=tex\n"));
        assert!(unit.contains("\nWantedBy=multi-user.target\n"));
        assert!(
            unit_text("/opt/my tex/tex-indexer")
                .contains("\nExecStart=\"/opt/my tex/tex-indexer\" service\n")
        );
    }

    #[test]
    fn packaged_unit_matches() {
        let packaged = include_str!("../../../installer/linux/tex-indexer.service");
        assert_eq!(
            packaged.replace("\r\n", "\n"),
            unit_text("/usr/lib/tex/tex-indexer")
        );
    }
}
