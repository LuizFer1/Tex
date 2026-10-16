pub type Record = u32;

pub const ROOT: Record = 5;

pub(crate) const IN_USE: u8 = 1;
pub(crate) const DIR: u8 = 2;
pub(crate) const HIDDEN: u8 = 4;
pub(crate) const SYSTEM: u8 = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attributes {
    pub dir: bool,
    pub hidden: bool,
    pub system: bool,
}

impl Attributes {
    pub(crate) fn to_flags(self) -> u8 {
        let mut flags = IN_USE;
        if self.dir {
            flags |= DIR;
        }
        if self.hidden {
            flags |= HIDDEN;
        }
        if self.system {
            flags |= SYSTEM;
        }
        flags
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Entry {
    pub parent: Record,
    pub name_offset: u32,
    pub name_len: u16,
    pub flags: u8,
}

const _: () = assert!(size_of::<Entry>() == 12);

impl Entry {
    pub fn in_use(&self) -> bool {
        self.flags & IN_USE != 0
    }

    pub fn is_dir(&self) -> bool {
        self.flags & DIR != 0
    }

    pub fn is_hidden(&self) -> bool {
        self.flags & HIDDEN != 0
    }

    pub fn is_system(&self) -> bool {
        self.flags & SYSTEM != 0
    }
}

/// MFT record number from an NTFS file reference (drops the 16-bit sequence number).
/// `None` for records that do not fit in 32 bits.
pub fn record_from_frn(frn: u64) -> Option<Record> {
    u32::try_from(frn & 0x0000_FFFF_FFFF_FFFF).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_from_frn_drops_the_sequence_number() {
        assert_eq!(record_from_frn(0x0003_0000_0000_0010), Some(16));
        assert_eq!(record_from_frn(0x0000_0001_0000_0000), None);
    }

    #[test]
    fn attributes_become_flags() {
        let entry = Entry {
            flags: Attributes {
                dir: true,
                hidden: true,
                system: false,
            }
            .to_flags(),
            ..Entry::default()
        };
        assert!(entry.in_use());
        assert!(entry.is_dir());
        assert!(entry.is_hidden());
        assert!(!entry.is_system());
    }
}
