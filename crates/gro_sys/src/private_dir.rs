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
//!
//! The made directory is held, for as long as its [`PrivateDir`] is, by a
//! shared `flock` on a descriptor reopened to read it (`O_RDONLY`, which an
//! `O_PATH` one can't be locked through), close-on-exec, so it's the
//! process's alone and goes when the process ends however it ends. Once
//! locked, the directory is checked to still have a link: one a sweep
//! removed between the make and the lock fails the make
//! ([`PrivateDirError::SweptBeforeLock`]) rather than leaving the process
//! a directory with no name.
//!
//! [`sweep_private_dirs`] removes the directories of processes gone: each
//! entry named as one is, a directory the process's effective user owns,
//! whose `/proc/<pid>/stat` is missing or names another start time (a
//! reused id), and which it can lock exclusively without waiting; removed
//! by [`Last::remove_tree`] while locked, which follows no symlink, so a
//! symlink a gone run left inside leads nowhere the sweep acts. A
//! directory its process still holds is kept whatever `/proc` shows, as it
//! shows nothing of a process `hidepid` hides or of one in another pid
//! namespace sharing the parent; so is another user's, a symlink, and
//! anything whose owner can't be read. It's availability, keeping a run's
//! leftovers from piling up, not a wall; that it follows no planted
//! symlink is part of the box's boundary.

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
    /// The directory, opened to read, holding its shared lock.
    _lock: File,
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
    /// The directory couldn't be locked.
    #[error("failed to lock {name}")]
    Lock {
        /// The name.
        name: String,
        /// The call's error.
        #[source]
        source: io::Error,
    },
    /// A sweep removed the directory between its make and its lock.
    #[error("{name} was swept before it was locked")]
    SweptBeforeLock {
        /// The name.
        name: String,
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
    let lock = lock(dir.as_fd(), false).map_err(|source| PrivateDirError::Lock {
        name: name.clone(),
        source,
    })?;
    let links = lock
        .metadata()
        .map_err(|source| PrivateDirError::Lock {
            name: name.clone(),
            source,
        })?
        .nlink();
    if links == 0 {
        return Err(PrivateDirError::SweptBeforeLock { name });
    }
    Ok(PrivateDir {
        dir: dir.into(),
        name,
        _lock: lock,
    })
}

/// The directory `dir` names, reopened to read through its `/proc/self/fd`
/// link (`O_RDONLY | O_DIRECTORY`), then locked: shared, waiting for an
/// exclusive lock to go, or `exclusive`, without waiting.
///
/// # Errors
///
/// Returns the reopen's error, or the lock's: `WouldBlock` for an exclusive
/// lock another holds a lock against.
fn lock(dir: BorrowedFd<'_>, exclusive: bool) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = beneath::through_proc(dir, |link| {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(link)
    })?;
    if exclusive {
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
            std::fs::TryLockError::Error(error) => error,
        })?;
    } else {
        file.lock_shared()?;
    }
    Ok(file)
}

/// What [`sweep_private_dirs`] did.
#[derive(Debug, Default)]
pub struct Swept {
    /// The names of the directories removed.
    pub removed: Vec<String>,
    /// The names of those whose removal failed, each with its error,
    /// left as the failure left them.
    pub failed: Vec<(String, io::Error)>,
}

/// Removes beneath `parent` the private directories of processes gone, as
/// the [module doc](self) says.
///
/// # Errors
///
/// Returns the error when `parent` can't be listed; a directory that
/// can't be removed is in [`Swept::failed`].
pub fn sweep_private_dirs(parent: BorrowedFd<'_>) -> io::Result<Swept> {
    let user = effective_uid();
    let mut swept = Swept::default();
    for entry in beneath::read_dir(parent, Path::new("."))? {
        let Some((name, pid, start)) = entry
            .name
            .to_str()
            .and_then(|name| parse_name(name).map(|(pid, start)| (name, pid, start)))
        else {
            continue;
        };
        let path = Path::new(name);
        // a symlink is refused, not followed
        let Ok(handle) = beneath::open(parent, path, OpenAs::Path, Links::Refused) else {
            continue;
        };
        // a failure here, like one to open a descriptor (`EMFILE`), skips the
        // entry, as any other of an entry's does
        let Ok(owner) = handle
            .try_clone()
            .and_then(|handle| File::from(handle).metadata())
        else {
            continue;
        };
        if !sweeps(owner.is_dir(), owner.uid(), user, start, running(pid)) {
            continue;
        }
        // held by its run, whatever `/proc` shows
        let Ok(_held) = lock(handle.as_fd(), true) else {
            continue;
        };
        match Last::open(parent, path).and_then(|last| last.remove_tree()) {
            Ok(()) => swept.removed.push(name.to_owned()),
            // another sweep's
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {}
            Err(error) => swept.failed.push((name.to_owned(), error)),
        }
    }
    Ok(swept)
}

/// The process id and start time a private directory's `name` holds, when
/// it's named as [`make_private_dir`] names one, each number as Rust
/// writes it, so `gro-007-5` names none.
fn parse_name(name: &str) -> Option<(u32, u64)> {
    let (pid, start) = name.strip_prefix(PRIVATE_DIR_PREFIX)?.split_once('-')?;
    let (pid_number, start_number) = (pid.parse::<u32>().ok()?, start.parse::<u64>().ok()?);
    (pid_number.to_string() == pid && start_number.to_string() == start)
        .then_some((pid_number, start_number))
}

/// Whether a process with an id is running, and since when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Running {
    /// No process has the id.
    None,
    /// One has, started at this time, in clock ticks since boot.
    Since(u64),
    /// Its `stat` couldn't be read or parsed.
    Unread,
}

/// Whether the process `pid` is running now, and since when.
fn running(pid: u32) -> Running {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => parse_start_time(&stat).map_or(Running::Unread, Running::Since),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Running::None,
        Err(_) => Running::Unread,
    }
}

/// Whether a private directory is swept: a directory (`dir`), owned by
/// `user`, the process's effective user, named for a process that started
/// at `start` and isn't `running` with that start: none has its id, or
/// another started since has it. One whose owner can't be read is kept.
const fn sweeps(dir: bool, owner: u32, user: u32, start: u64, running: Running) -> bool {
    dir && owner == user
        && match running {
            Running::None => true,
            Running::Since(now) => now != start,
            Running::Unread => false,
        }
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
    /// A directory is swept only when it's one, the process's user owns
    /// it, and the process it's named for isn't running with its start:
    /// gone, or its id reused by another; another user's, a live owner's,
    /// and one whose owner can't be read are kept.
    #[test]
    fn a_gone_owner_s_directory_is_swept() {
        use super::Running;
        for (case, dir, owner, running, swept) in [
            ("gone", true, 1000, Running::None, true),
            ("a reused id", true, 1000, Running::Since(9), true),
            ("live", true, 1000, Running::Since(5), false),
            ("unreadable", true, 1000, Running::Unread, false),
            ("another user's", true, 1001, Running::None, false),
            ("not a directory", false, 1000, Running::None, false),
        ] {
            assert_eq!(super::sweeps(dir, owner, 1000, 5, running), swept, "{case}");
        }
    }

    /// A name is a private directory's only as `make_private_dir` writes
    /// one.
    #[test]
    fn only_a_private_directory_s_name_parses() {
        assert_eq!(super::parse_name("gro-42-7"), Some((42, 7)));
        for name in [
            "gro-42",
            "gro-007-7",
            "gro-+4-7",
            "gro-4-",
            "gro--4-7",
            "x-4-7",
            "gro-4-7-8",
        ] {
            assert_eq!(super::parse_name(name), None, "{name}");
        }
    }

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
