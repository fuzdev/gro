//! An exclusive lock on a directory: an `flock` on its [`LOCK_FILE`], which
//! the kernel releases when the file closes, on drop or on any exit, so no
//! lock outlives its process.
//!
//! The holder writes its process id into the file, for a refused caller to
//! name. The file is opened never through a symlink at its path
//! (`O_NOFOLLOW`), and kept only as a regular file of one link: a symlink
//! planted there fails the lock (`ELOOP`, "Too many levels of symbolic
//! links"), and a hard link, a directory, or a FIFO fails it too, rather
//! than have the id written into, or the holder read from, another file.
//!
//! A lock outlives its drop for a moment when another thread of the process
//! is spawning a child: the child holds a copy of every descriptor until its
//! `exec` closes them, the lock's among them. So a refusing lock retries for
//! [`GRACE`] before it refuses.

use std::fs::{self, File, TryLockError};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The file in a locked directory that carries the lock.
pub const LOCK_FILE: &str = "lock";

/// How long [`try_lock_dir`] retries a held lock before it refuses: far
/// past a spawning child's `exec`, far below a run.
pub const GRACE: Duration = Duration::from_secs(1);

/// The pause between retries.
const RETRY: Duration = Duration::from_millis(10);

/// The most bytes of a held lock file read for its holder's id, which a
/// process id's digits fit many times over.
const HOLDER_LIMIT: u64 = 32;

/// A held lock on a directory, released on drop.
#[derive(Debug)]
pub struct DirLock {
    _file: File,
}

/// A filesystem call that failed taking a lock: the directory or its lock
/// file couldn't be made, opened, locked, or written.
#[derive(Debug, thiserror::Error)]
#[error("{}: {source}", path.display())]
pub struct LockIoError {
    /// The directory or the lock file.
    pub path: PathBuf,
    /// The failure.
    #[source]
    pub source: io::Error,
}

/// Why [`try_lock_dir`] couldn't lock a directory.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another process holds it.
    #[error(
        "{} is locked by process {}",
        path.display(),
        holder.as_deref().unwrap_or("unknown")
    )]
    Held {
        /// The lock file.
        path: PathBuf,
        /// The process id the holder wrote, when it wrote one.
        holder: Option<String>,
    },
    /// A filesystem call failed.
    #[error(transparent)]
    Io(#[from] LockIoError),
}

/// Locks `dir` (made if missing) by its [`LOCK_FILE`], waiting until
/// another process holding it releases it, and writes this process's id
/// into the file.
///
/// # Errors
///
/// Fails when a filesystem call fails.
pub fn lock_dir_blocking(dir: &Path) -> Result<DirLock, LockIoError> {
    let (path, file) = open_lock(dir)?;
    file.lock().map_err(|source| LockIoError {
        path: path.clone(),
        source,
    })?;
    hold(path, file)
}

/// Locks `dir` (made if missing) by its [`LOCK_FILE`], refusing once another
/// process still holds it after [`GRACE`], and writes this process's id
/// into the file.
///
/// # Errors
///
/// Fails with [`LockError::Held`] when another holds it, or [`LockError::Io`]
/// when a filesystem call fails.
pub fn try_lock_dir(dir: &Path) -> Result<DirLock, LockError> {
    let (path, file) = open_lock(dir)?;
    let deadline = Instant::now() + GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(RETRY);
            }
            Err(TryLockError::WouldBlock) => {
                let mut holder = String::new();
                if (&file)
                    .take(HOLDER_LIMIT)
                    .read_to_string(&mut holder)
                    .is_err()
                {
                    holder.clear();
                }
                let holder = holder.trim();
                return Err(LockError::Held {
                    holder: (!holder.is_empty()).then(|| holder.to_owned()),
                    path,
                });
            }
            Err(TryLockError::Error(source)) => return Err(LockIoError { path, source }.into()),
        }
    }
    Ok(hold(path, file)?)
}

/// Makes `dir` if missing and opens its lock file, unlocked, returning its
/// path and the file: a symlink at the file's path fails the open
/// (`ELOOP`), and anything opened but a regular file of one link is refused
/// with [`io::ErrorKind::InvalidInput`].
fn open_lock(dir: &Path) -> Result<(PathBuf, File), LockIoError> {
    fs::create_dir_all(dir).map_err(|source| LockIoError {
        path: dir.to_owned(),
        source,
    })?;
    let path = dir.join(LOCK_FILE);
    let opened = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .and_then(|file| {
            let metadata = file.metadata()?;
            if metadata.is_file() && metadata.nlink() == 1 {
                Ok(file)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the lock file isn't a regular file of one link, so it isn't taken",
                ))
            }
        });
    match opened {
        Ok(file) => Ok((path, file)),
        Err(source) => Err(LockIoError { path, source }),
    }
}

/// The lock on the locked `file` at `path`, once this process's id is
/// written into it.
fn hold(path: PathBuf, mut file: File) -> Result<DirLock, LockIoError> {
    match file
        .set_len(0)
        .and_then(|()| writeln!(file, "{}", std::process::id()))
    {
        Ok(()) => Ok(DirLock { _file: file }),
        Err(source) => Err(LockIoError { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_refused_until_dropped() {
        let dir = std::env::temp_dir().join(format!("gro_bench_lock_{}", std::process::id()));
        let held = try_lock_dir(&dir).unwrap();
        // a second open file can't take it while the first holds it
        let refused = try_lock_dir(&dir);
        let text = fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
        drop(held);
        let again = try_lock_dir(&dir).is_ok();
        fs::remove_dir_all(&dir).unwrap();
        let pid = std::process::id().to_string();
        assert!(
            matches!(refused, Err(LockError::Held { ref holder, .. }) if holder.as_deref() == Some(pid.as_str())),
            "{refused:?}"
        );
        assert_eq!(text.trim(), pid);
        assert!(again);
    }

    #[test]
    fn a_symlink_at_the_lock_file_is_refused_never_followed() {
        let base = std::env::temp_dir().join(format!("gro_bench_lock_link_{}", std::process::id()));
        let dir = base.join("locked");
        fs::create_dir_all(&dir).unwrap();
        let target = base.join("outside");
        fs::write(&target, "kept\n").unwrap();
        std::os::unix::fs::symlink(&target, dir.join(LOCK_FILE)).unwrap();
        // each lock dropped at once: one taken through the link would hold
        // the target, and the next call would wait on it
        let tried = try_lock_dir(&dir).map(drop);
        let blocking = lock_dir_blocking(&dir).map(drop);
        let kept = fs::read_to_string(&target).unwrap();
        fs::remove_dir_all(&base).unwrap();
        let is_loop = |error: &LockIoError| {
            error.path == dir.join(LOCK_FILE) && error.source.raw_os_error() == Some(libc::ELOOP)
        };
        assert!(
            matches!(tried, Err(LockError::Io(ref error)) if is_loop(error)),
            "{tried:?}"
        );
        assert!(
            matches!(blocking, Err(ref error) if is_loop(error)),
            "{blocking:?}"
        );
        assert_eq!(kept, "kept\n");
    }

    /// A held lock's holder is read no further than [`HOLDER_LIMIT`]: what
    /// a holder wrote past it is never read.
    #[test]
    fn a_held_lock_s_holder_is_read_to_its_limit() {
        let dir = std::env::temp_dir().join(format!("gro_bench_lock_long_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(LOCK_FILE), "7".repeat(64)).unwrap();
        let holder = File::open(dir.join(LOCK_FILE)).unwrap();
        holder.lock().unwrap();
        let refused = try_lock_dir(&dir).map(drop);
        drop(holder);
        fs::remove_dir_all(&dir).unwrap();
        let limit = usize::try_from(HOLDER_LIMIT).unwrap();
        assert!(
            matches!(refused, Err(LockError::Held { ref holder, .. }) if holder.as_deref() == Some("7".repeat(limit).as_str())),
            "{refused:?}"
        );
    }

    /// Whether `result` is the refusal of a lock file that isn't a regular
    /// file of one link, on `dir`'s lock path.
    fn is_not_a_lock_file<T>(result: &Result<T, LockIoError>, dir: &Path) -> bool {
        result.as_ref().is_err_and(|error| {
            error.path == dir.join(LOCK_FILE) && error.source.kind() == io::ErrorKind::InvalidInput
        })
    }

    #[test]
    fn a_hard_link_at_the_lock_file_is_refused_never_written() {
        let base = std::env::temp_dir().join(format!("gro_bench_lock_hard_{}", std::process::id()));
        let dir = base.join("locked");
        fs::create_dir_all(&dir).unwrap();
        let target = base.join("outside");
        fs::write(&target, "kept\n").unwrap();
        fs::hard_link(&target, dir.join(LOCK_FILE)).unwrap();
        // each lock dropped at once, as above
        let tried = try_lock_dir(&dir).map(drop).map_err(|error| match error {
            LockError::Io(error) => error,
            LockError::Held { .. } => panic!("{error:?}"),
        });
        let blocking = lock_dir_blocking(&dir).map(drop);
        let kept = fs::read_to_string(&target).unwrap();
        fs::remove_dir_all(&base).unwrap();
        assert!(is_not_a_lock_file(&tried, &dir), "{tried:?}");
        assert!(is_not_a_lock_file(&blocking, &dir), "{blocking:?}");
        assert_eq!(kept, "kept\n");
    }

    /// A directory at the lock path fails its open, and a FIFO there, even
    /// one another holder has locked, is refused before its holder would be
    /// read from it, which would wait forever for a writer: each refused
    /// within a bound.
    #[test]
    fn a_directory_or_fifo_at_the_lock_file_is_refused() {
        let base =
            std::env::temp_dir().join(format!("gro_bench_lock_kinds_{}", std::process::id()));
        let directory = base.join("directory");
        fs::create_dir_all(directory.join(LOCK_FILE)).unwrap();
        let fifo = base.join("fifo");
        fs::create_dir_all(&fifo).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(fifo.join(LOCK_FILE))
            .status()
            .unwrap();
        assert!(made.success(), "mkfifo: {made}");
        // the FIFO's holder: opened to read and write, which doesn't wait
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fifo.join(LOCK_FILE))
            .unwrap();
        holder.lock().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let (directory_dir, fifo_dir) = (directory.clone(), fifo.clone());
        std::thread::spawn(move || {
            let directory = try_lock_dir(&directory_dir).map(drop);
            let fifo = try_lock_dir(&fifo_dir).map(drop);
            let _ = send.send((directory, fifo));
        });
        let outcome = receive.recv_timeout(Duration::from_secs(30));
        drop(holder);
        fs::remove_dir_all(&base).unwrap();
        let (directory_result, fifo_result) = outcome.expect("the lock returns within 30 s");
        assert!(
            matches!(directory_result, Err(LockError::Io(ref error))
                if error.path == directory.join(LOCK_FILE)
                    && error.source.kind() == io::ErrorKind::IsADirectory),
            "{directory_result:?}"
        );
        let fifo_result = fifo_result.map_err(|error| match error {
            LockError::Io(error) => error,
            LockError::Held { .. } => panic!("{error:?}"),
        });
        assert!(is_not_a_lock_file(&fifo_result, &fifo), "{fifo_result:?}");
    }
}
