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

/// How long [`Wait::Refuse`] retries a held lock before it refuses: far past
/// a spawning child's `exec`, far below a run.
pub const GRACE: Duration = Duration::from_secs(1);

/// The pause between retries.
const RETRY: Duration = Duration::from_millis(10);

/// A held lock on a directory, released on drop.
#[derive(Debug)]
pub struct DirLock {
    _file: File,
}

/// Whether [`lock_dir`] waits for a lock another process holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// Wait until the holder releases it.
    Block,
    /// Refuse, with [`LockError::Held`], once it's still held after
    /// [`GRACE`].
    Refuse,
}

/// Why a directory couldn't be locked.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another process holds it ([`Wait::Refuse`] only).
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
    /// The directory or its lock file couldn't be made, opened, locked, or
    /// written.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Locks `dir` (made if missing) by its [`LOCK_FILE`], waiting or not as
/// `wait` says, and writes this process's id into the file.
///
/// # Errors
///
/// Fails with [`LockError::Held`] when another holds it and `wait` is
/// [`Wait::Refuse`], or [`LockError::Io`] when a filesystem call fails.
pub fn lock_dir(dir: &Path, wait: Wait) -> Result<DirLock, LockError> {
    let path = dir.join(LOCK_FILE);
    let io_error = |source| LockError::Io {
        path: path.clone(),
        source,
    };
    fs::create_dir_all(dir).map_err(|source| LockError::Io {
        path: dir.to_owned(),
        source,
    })?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(io_error)?;
    match wait {
        Wait::Block => file.lock().map_err(io_error)?,
        Wait::Refuse => {
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
                    Err(TryLockError::Error(source)) => return Err(io_error(source)),
                }
            }
        }
    }
    file.set_len(0)
        .and_then(|()| writeln!(file, "{}", std::process::id()))
        .map_err(io_error)?;
    Ok(DirLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_refused_until_dropped() {
        let dir = std::env::temp_dir().join(format!("gro_bench_lock_{}", std::process::id()));
        let held = lock_dir(&dir, Wait::Refuse).unwrap();
        // a second open file can't take it while the first holds it
        let refused = lock_dir(&dir, Wait::Refuse);
        let text = fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
        drop(held);
        let again = lock_dir(&dir, Wait::Refuse).is_ok();
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
