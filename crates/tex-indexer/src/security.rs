use std::io;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

use crate::wide;

const PRIVATE_DIR_SDDL: &str = "D:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)";

pub(crate) struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut descriptor = ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(sddl).as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

/// Creates `path` readable only by SYSTEM and Administrators. An existing folder is left as is.
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    let descriptor = SecurityDescriptor::from_sddl(PRIVATE_DIR_SDDL)?;
    let attributes = descriptor.attributes();
    let path = wide(&path.to_string_lossy());
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0
        && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
