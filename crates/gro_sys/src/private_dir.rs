//! A directory private to the process, made beneath a directory's
//! descriptor, for what the process and its children write as temporary
//! files.
//!
//! [`make_private_dir`] names it [`PRIVATE_DIR_PREFIX`] then
//! `<pid>-<start>`: the process's id and its start time, in clock ticks
//! since boot, as `/proc/self/stat` gives it. The two name one process
//! while the machine runs, a reused id never with the same start, so a
//! sweep of stale directories can tell a live owner from a gone one by
//! `/proc/<pid>/stat`.
//!
//! The directory is made by `mkdirat` beneath the descriptor, which refuses
//! any entry already at the name, a symlink, dangling or not, among them,
//! then opened by that one name with no symlink followed, held by its
//! descriptor, checked to be a directory the process's effective user
//! owns, and set to mode `0700` whatever the umask left, through the
//! descriptor's `/proc/self/fd` link, so `/proc` must be reachable. In a
//! shared directory with the sticky bit (`/tmp`), another user can neither
//! plant the name unseen (the make fails) nor move the directory once
//! made.

use std::fs::{File, Permissions};
use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

use crate::beneath::{self, Last, Links, OpenAs};

/// What the name of a directory [`make_private_dir`] makes starts with.
pub const PRIVATE_DIR_PREFIX: &str = "gro-";

/// The mode a private directory has: its owner's alone.
const PRIVATE_DIR_MODE: u32 = 0o700;

/// A directory [`make_private_dir`] made, held by its descriptor.
#[derive(Debug)]
pub struct PrivateDir {
    /// The directory, opened as a path only (`O_PATH | O_DIRECTORY`), to
    /// resolve beneath and to name, as [`OpenAs::Directory`] opens one.
    dir: OwnedFd,
    /// Its name in the directory it was made in.
    name: String,
}

impl PrivateDir {
    /// The directory's descriptor.
    pub fn dir(&self) -> BorrowedFd<'_> {
        self.dir.as_fd()
    }

    /// Its name in the directory it was made in.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Why [`make_private_dir`] failed.
#[derive(Debug, thiserror::Error)]
pub enum PrivateDirError {
    /// The process's start time couldn't be read from `/proc/self/stat`;
    /// nothing was made.
    #[error("failed to read the process's start time")]
    StartUnread(#[source] io::Error),
    /// An entry of any kind is at the name already; nothing was made.
    #[error("{name} exists already")]
    Exists {
        /// The name.
        name: String,
    },
    /// `mkdirat` failed otherwise; nothing was made.
    #[error("failed to make {name}")]
    Make {
        /// The name.
        name: String,
        /// The call's error.
        #[source]
        source: io::Error,
    },
    /// The directory made couldn't be opened by its name: something
    /// replaced it, a symlink (`ELOOP`) or not a directory (`ENOTDIR`).
    #[error("failed to open {name}, made just before")]
    Open {
        /// The name.
        name: String,
        /// The call's error.
        #[source]
        source: io::Error,
    },
    /// What the name opened isn't a directory the process's effective user
    /// owns.
    #[error("{name} is no directory the process's user owns, though it made it")]
    Foreign {
        /// The name.
        name: String,
    },
    /// Its mode couldn't be read or set.
    #[error("failed to set {name}'s mode")]
    Mode {
        /// The name.
        name: String,
        /// The call's error.
        #[source]
        source: io::Error,
    },
}

/// Makes a directory private to the process beneath `parent`, as the
/// [module doc](self) says, and returns it held by its descriptor.
///
/// # Errors
///
/// Returns the step that failed ([`PrivateDirError`]):
/// [`PrivateDirError::Exists`] when any entry is at the name, nothing
/// made. Once made, a later step's failure leaves the empty directory
/// there.
pub fn make_private_dir(parent: BorrowedFd<'_>) -> Result<PrivateDir, PrivateDirError> {
    let start = start_time().map_err(PrivateDirError::StartUnread)?;
    let name = format!("{PRIVATE_DIR_PREFIX}{}-{start}", std::process::id());
    let path = Path::new(&name);
    // a name without a `/`, acted on in `parent` itself
    match Last::open(parent, path).and_then(|last| last.mkdir(PRIVATE_DIR_MODE)) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            return Err(PrivateDirError::Exists { name });
        }
        Err(source) => return Err(PrivateDirError::Make { name, source }),
    }
    // opened as a path only, which takes no permission on the directory, so
    // a umask that took the owner's bits off doesn't refuse it
    let dir = beneath::open(parent, path, OpenAs::Directory, Links::Refused)
        .map(File::from)
        .map_err(|source| PrivateDirError::Open {
            name: name.clone(),
            source,
        })?;
    let metadata = dir.metadata().map_err(|source| PrivateDirError::Mode {
        name: name.clone(),
        source,
    })?;
    if !metadata.is_dir() || metadata.uid() != effective_uid() {
        return Err(PrivateDirError::Foreign { name });
    }
    // `mkdirat` took the umask's bits off the mode, which may have been the
    // owner's own; set through the descriptor's `/proc/self/fd` link, as a
    // descriptor opened as a path changes no mode itself
    beneath::through_proc(dir.as_fd(), |link| {
        std::fs::set_permissions(link, Permissions::from_mode(PRIVATE_DIR_MODE))
    })
    .map_err(|source| PrivateDirError::Mode {
        name: name.clone(),
        source,
    })?;
    Ok(PrivateDir {
        dir: dir.into(),
        name,
    })
}

/// The process's effective user id.
#[expect(
    unsafe_code,
    reason = "std has no effective user id, and a libc call is `unsafe`"
)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes nothing, always succeeds, and touches no
    // memory of the process's
    unsafe { libc::geteuid() }
}

/// The process's start time, in clock ticks since boot, from
/// `/proc/self/stat`.
fn start_time() -> io::Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    parse_start_time(&stat).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "/proc/self/stat has no start time",
        )
    })
}

/// The start time in a `/proc/<pid>/stat` line: its 22nd field, the 20th
/// after the command's name, which closes with the line's last `)` and may
/// hold spaces and parentheses of its own.
fn parse_start_time(stat: &str) -> Option<u64> {
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(test)]
mod tests {
    /// The start time is the 22nd field, counted past a name holding
    /// spaces and parentheses; a line too short has none.
    #[test]
    fn the_start_time_is_the_22nd_field() {
        let fields: Vec<String> = (3..=25).map(|field| field.to_string()).collect();
        let line = format!("42 (a) (b c)) {}\n", fields.join(" "));
        assert_eq!(super::parse_start_time(&line), Some(22));
        assert_eq!(super::parse_start_time("42 (a) 3 4"), None);
        assert_eq!(super::parse_start_time("42 a 3 4"), None);
    }
}
