/// Owner and permission bits of a directory on a Unix volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    /// The permission bits of `st_mode`, such as `0o755`.
    pub mode: u16,
}
