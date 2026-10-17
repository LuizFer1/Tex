use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

/// Encrypts `data` with DPAPI so that only the current Windows user can decrypt it.
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(data)?;
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    take(ok, output)
}

/// Decrypts data made by [`protect`] for the current Windows user.
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(data)?;
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    take(ok, output)
}

fn blob(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let length = u32::try_from(data.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "data too large for DPAPI"))?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: data.as_ptr() as *mut u8,
    })
}

fn take(ok: i32, output: CRYPT_INTEGER_BLOB) -> io::Result<Vec<u8>> {
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let bytes = if output.pbData.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec()
    };
    unsafe { LocalFree(output.pbData as _) };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_for_the_current_user() {
        let secret = b"tex clipboard key 0123456789abcdef";
        let sealed = protect(secret).unwrap();
        assert_ne!(sealed.as_slice(), secret.as_slice());
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }

    #[test]
    fn rejects_garbage() {
        assert!(unprotect(b"not a dpapi blob").is_err());
    }
}
