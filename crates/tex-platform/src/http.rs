use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
};

const CHUNK: usize = 64 * 1024;

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> io::Result<Handle> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// GET over HTTPS through the system proxy, following redirects. Fails on any status but
/// 200 or when the body passes `max_bytes`.
pub fn http_get(url: &str, max_bytes: u64) -> io::Result<Vec<u8>> {
    let (host, port, path) = split_url(url)?;
    let agent = wide(&format!("Tex/{}", env!("CARGO_PKG_VERSION")));
    let session = Handle::new(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        )
    })?;
    let host = wide(&host);
    let connection = Handle::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), port, 0) })?;
    let verb = wide("GET");
    let path = wide(&path);
    let request = Handle::new(unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    })?;
    let headers = wide("Accept: application/vnd.github+json, */*\r\n");
    let sent =
        unsafe { WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, ptr::null(), 0, 0, 0) };
    if sent == 0 || unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut status: u32 = 0;
    let mut length = size_of::<u32>() as u32;
    let queried = unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            (&raw mut status).cast(),
            &mut length,
            ptr::null_mut(),
        )
    };
    if queried == 0 {
        return Err(io::Error::last_os_error());
    }
    if status != 200 {
        return Err(io::Error::other(format!("HTTP {status}")));
    }
    let mut body = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut read = 0u32;
        let ok = unsafe {
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                CHUNK as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Ok(body);
        }
        if body.len() as u64 + u64::from(read) > max_bytes {
            return Err(io::Error::other("response too large"));
        }
        body.extend_from_slice(&chunk[..read as usize]);
    }
}

fn split_url(url: &str) -> io::Result<(String, u16, String)> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not an https URL: {url}"),
        )
    };
    let rest = url.strip_prefix("https://").ok_or_else(invalid)?;
    let (authority, path) = rest
        .find('/')
        .map_or((rest, "/"), |at| (&rest[..at], &rest[at..]));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().map_err(|_| invalid())?),
        None => (authority, 443),
    };
    if host.is_empty() {
        return Err(invalid());
    }
    Ok((host.to_owned(), port, path.to_owned()))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_https_urls() {
        assert_eq!(
            split_url("https://api.github.com/repos/LuizFer1/Tex/releases/latest").unwrap(),
            (
                "api.github.com".to_owned(),
                443,
                "/repos/LuizFer1/Tex/releases/latest".to_owned()
            )
        );
        assert_eq!(
            split_url("https://example.test:8443").unwrap(),
            ("example.test".to_owned(), 8443, "/".to_owned())
        );
        assert_eq!(split_url("https://example.test/a?b=c").unwrap().2, "/a?b=c");
    }

    #[test]
    fn refuses_anything_but_https() {
        for bad in [
            "http://example.test/",
            "ftp://example.test/",
            "https:///path",
            "https://example.test:port/",
            "example.test",
        ] {
            assert!(split_url(bad).is_err(), "{bad}");
        }
    }
}
