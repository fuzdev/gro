//! An exclusive lock on a directory: an `flock` on its [`LOCK_FILE`], which
//! the kernel releases when the file closes, on drop or on any exit, so no
//! lock outlives its process.
//!
//! The holder writes its process id into the file, for a refused caller to
//! name.
//!
//! A lock outlives its drop for a moment when another thread of the process
//! is spawning a child: the child holds a copy of every descriptor until its
//! `exec` closes them, the lock's among them. So a refusing lock retries for
//! [`GRACE`] before it refuses.

use std::fs::{self, File, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The file in a locked directory that carries the lock.
pub const LOCK_FILE: &str = "lock";

/// How long [`try_lock_dir`] retries a held lock before it refuses: far
/// past a spawning child's `exec`, far below a run.
pub const GRACE: Duration = Duration::from_secs(1);

/// The pause between retries.
const RETRY: Duration = Duration::from_millis(10);

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
                let holder = fs::read_to_string(&path).unwrap_or_default();
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
/// path and the file.
fn open_lock(dir: &Path) -> Result<(PathBuf, File), LockIoError> {
    fs::create_dir_all(dir).map_err(|source| LockIoError {
        path: dir.to_owned(),
        source,
    })?;
    let path = dir.join(LOCK_FILE);
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
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
}
