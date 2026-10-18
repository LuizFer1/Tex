use std::io;

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

const USER_AGENT: &str = concat!("Tex/", env!("CARGO_PKG_VERSION"));

/// GET over HTTPS with the system's certificates and the proxy in `HTTPS_PROXY`, following
/// redirects (HTTPS only). Fails on any status but 200 or when the body passes `max_bytes`.
pub fn http_get(url: &str, max_bytes: u64) -> io::Result<Vec<u8>> {
    if !is_https(url) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not an https URL: {url}"),
        ));
    }
    let mut response = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json, */*")
        .call()
        .map_err(ureq::Error::into_io)?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(io::Error::other(format!("HTTP {status}")));
    }
    response
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(ureq::Error::into_io)
}

fn agent() -> Agent {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let config = Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .tls_config(tls)
        .build();
    Agent::new_with_config(config)
}

fn is_https(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST: &str = "https://api.github.com/repos/LuizFer1/Tex/releases/latest";

    #[test]
    fn only_https_urls_are_fetched() {
        assert!(is_https(LATEST));
        for bad in [
            "http://example.test/",
            "ftp://example.test/",
            "https:///path",
            "https://",
            "example.test",
        ] {
            assert!(!is_https(bad), "{bad}");
            assert_eq!(
                http_get(bad, 10).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad}"
            );
        }
    }

    #[test]
    #[ignore = "goes online"]
    fn reads_the_latest_release_from_github() {
        let body = http_get(LATEST, 1024 * 1024).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"tag_name\""));
        assert!(http_get(LATEST, 10).is_err());
        assert!(
            http_get(
                "https://api.github.com/repos/LuizFer1/Tex/no-such-page",
                1024 * 1024
            )
            .is_err()
        );
    }
}
