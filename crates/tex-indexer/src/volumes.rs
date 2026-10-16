use std::ffi::c_void;
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_JOURNAL_NOT_ACTIVE, GENERIC_READ, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetLogicalDriveStringsW,
    GetVolumeInformationW, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    CREATE_USN_JOURNAL_DATA, FSCTL_CREATE_USN_JOURNAL, FSCTL_ENUM_USN_DATA,
    FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0,
    NTFS_VOLUME_DATA_BUFFER, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

use crate::wide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtfsVolume {
    pub letter: char,
    pub serial: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Journal {
    pub id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
}

/// Fixed and removable drives with a letter whose file system is NTFS. Works without admin.
pub fn ntfs_volumes() -> Vec<NtfsVolume> {
    let mut buffer = [0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) } as usize;
    buffer[..len.min(buffer.len())]
        .split(|&unit| unit == 0)
        .filter(|root| !root.is_empty())
        .filter_map(|root| {
            let mut root = root.to_vec();
            root.push(0);
            let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
            if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
                return None;
            }
            let mut serial = 0u32;
            let mut file_system = [0u16; 32];
            let ok = unsafe {
                GetVolumeInformationW(
                    root.as_ptr(),
                    ptr::null_mut(),
                    0,
                    &mut serial,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    file_system.as_mut_ptr(),
                    file_system.len() as u32,
                )
            };
            let end = file_system
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(file_system.len());
            if ok == 0 || String::from_utf16_lossy(&file_system[..end]) != "NTFS" {
                return None;
            }
            let letter = char::from_u32(u32::from(root[0]))?.to_ascii_uppercase();
            Some(NtfsVolume { letter, serial })
        })
        .collect()
}

pub fn windows_dir() -> Option<String> {
    let mut buffer = [0u16; 260];
    let len = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    (len > 0 && len < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..len]))
}

/// Read handle to a whole volume (`\\.\C:`). Opening it requires administrator rights.
pub struct VolumeHandle(HANDLE);

unsafe impl Send for VolumeHandle {}

impl VolumeHandle {
    pub fn open(letter: char) -> io::Result<Self> {
        let path = wide(&format!(r"\\.\{letter}:"));
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    /// The volume's USN journal, created with default sizes when it does not exist yet.
    pub fn journal(&self) -> io::Result<Journal> {
        match self.query_journal() {
            Err(error) if error.raw_os_error() == Some(ERROR_JOURNAL_NOT_ACTIVE as i32) => {
                let create = CREATE_USN_JOURNAL_DATA {
                    MaximumSize: 0,
                    AllocationDelta: 0,
                };
                self.control(
                    FSCTL_CREATE_USN_JOURNAL,
                    (&create as *const CREATE_USN_JOURNAL_DATA).cast(),
                    size_of::<CREATE_USN_JOURNAL_DATA>(),
                    ptr::null_mut(),
                    0,
                )?;
                self.query_journal()
            }
            other => other,
        }
    }

    /// Number of MFT records, used only to report indexing progress.
    pub fn record_count(&self) -> io::Result<u64> {
        let mut data: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
        self.control(
            FSCTL_GET_NTFS_VOLUME_DATA,
            ptr::null(),
            0,
            (&mut data as *mut NTFS_VOLUME_DATA_BUFFER).cast(),
            size_of::<NTFS_VOLUME_DATA_BUFFER>(),
        )?;
        Ok(data.MftValidDataLength as u64 / u64::from(data.BytesPerFileRecordSegment.max(1)))
    }

    /// Calls `on_batch` with each raw FSCTL_ENUM_USN_DATA output until every MFT record was listed.
    pub fn enumerate(&self, high_usn: i64, mut on_batch: impl FnMut(&[u8])) -> io::Result<()> {
        let mut buffer = vec![0u64; 128 * 1024];
        let mut input = MFT_ENUM_DATA_V0 {
            StartFileReferenceNumber: 0,
            LowUsn: 0,
            HighUsn: high_usn,
        };
        loop {
            let returned = match self.control(
                FSCTL_ENUM_USN_DATA,
                (&input as *const MFT_ENUM_DATA_V0).cast(),
                size_of::<MFT_ENUM_DATA_V0>(),
                buffer.as_mut_ptr().cast(),
                buffer.len() * 8,
            ) {
                Ok(returned) => returned,
                Err(error) if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) => {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let bytes = &as_bytes(&buffer)[..returned];
            if returned <= 8 {
                return Ok(());
            }
            input.StartFileReferenceNumber = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            on_batch(bytes);
        }
    }

    /// Blocks until the journal has close records after `start_usn`, then returns the raw output.
    pub fn read_journal<'a>(
        &self,
        journal_id: u64,
        start_usn: i64,
        buffer: &'a mut [u64],
    ) -> io::Result<&'a [u8]> {
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: start_usn,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 1,
            Timeout: 0,
            BytesToWaitFor: 1,
            UsnJournalID: journal_id,
        };
        let returned = self.control(
            FSCTL_READ_USN_JOURNAL,
            (&input as *const READ_USN_JOURNAL_DATA_V0).cast(),
            size_of::<READ_USN_JOURNAL_DATA_V0>(),
            buffer.as_mut_ptr().cast(),
            buffer.len() * 8,
        )?;
        Ok(&as_bytes(buffer)[..returned])
    }

    fn query_journal(&self) -> io::Result<Journal> {
        let mut data: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
        self.control(
            FSCTL_QUERY_USN_JOURNAL,
            ptr::null(),
            0,
            (&mut data as *mut USN_JOURNAL_DATA_V0).cast(),
            size_of::<USN_JOURNAL_DATA_V0>(),
        )?;
        Ok(Journal {
            id: data.UsnJournalID,
            first_usn: data.FirstUsn,
            next_usn: data.NextUsn,
        })
    }

    fn control(
        &self,
        code: u32,
        input: *const c_void,
        input_len: usize,
        output: *mut c_void,
        output_len: usize,
    ) -> io::Result<usize> {
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                code,
                input,
                input_len as u32,
                output,
                output_len as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(returned as usize)
    }
}

impl Drop for VolumeHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn as_bytes(buffer: &[u64]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast(), buffer.len() * 8) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_system_drive() {
        let system = windows_dir()
            .unwrap()
            .chars()
            .next()
            .unwrap()
            .to_ascii_uppercase();
        let volumes = ntfs_volumes();
        assert!(
            volumes.iter().any(|volume| volume.letter == system),
            "{volumes:?}"
        );
        assert!(
            volumes
                .iter()
                .all(|volume| volume.letter.is_ascii_uppercase())
        );
    }

    #[test]
    fn windows_dir_is_absolute() {
        let dir = windows_dir().unwrap();
        assert_eq!(&dir[1..3], r":\");
    }
}
