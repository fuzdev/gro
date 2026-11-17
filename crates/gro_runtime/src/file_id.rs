//! A file's identity ([`FileId`]), which the resolver takes as it finds a
//! file, and the module loader and the fs host module hold a file they open to.

/// A file's identity, its device and inode numbers and its birth time, taken
/// when the resolver looked it up, or, for a directory the fs host module
/// is granted, when the embedder checked it (`FsGrant`).
///
/// The module loader opens the path and refuses the file when the one it
/// opened has another, so a file or a directory on its path swapped between
/// resolving and reading can't substitute another file, except a file remade
/// with no birth time to tell it apart (see [`FileId::born`]); the fs host
/// module holds a granted directory's root to it the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    /// The device holding the file.
    pub dev: u64,
    /// The file's inode number on that device.
    pub ino: u64,
    /// When the inode was made, where the filesystem records it: ext4 hands
    /// a file made after one is removed the freed inode number, so the
    /// number alone can't tell them apart. `None` where the filesystem
    /// records no birth time, and then a remade file that reuses the inode
    /// isn't caught, nor is one made within the same tick of the
    /// filesystem's clock as the original.
    pub born: Option<std::time::SystemTime>,
}

impl FileId {
    /// The identity of the file `metadata` was read from.
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            born: metadata.created().ok(),
        }
    }
}
