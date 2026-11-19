//! Pins: what a program's name resolves to, by the Node module's rules,
//! held for the run so a later change on disk never changes which file a
//! handle names.
//!
//! A bare name walks the `PATH` the embedder read for children: absolute
//! entries only, so an empty or relative one can't reach the working
//! directory, skipping each entry spelled as a `node_modules/.bin`
//! directory, to the first regular file there it may execute, a stat that
//! fails reading as nothing there, as `execvp` walks past it. A name
//! holding `/` resolves against the project's root by its text: absolute,
//! or climbing above the root by `..`, it's `not_granted`, and one ending
//! in `/` or `/.` names no file. A `.bat` or `.cmd` name or pin is
//! `not_executable`, as what `cmd.exe` would interpret.
//!
//! What's found is pinned at its real path, opened as a path only
//! (`O_PATH`): the pin's identity is that descriptor's file, and its first
//! bytes are read through it (`/proc/self/fd`), so the file judged is the
//! file opened, however its path changes between. A file whose first bytes
//! are `#!` is a script, whose pin keeps no descriptor: it's run by the
//! path it was found at, which must still lead to its pin's identity when
//! it's launched. One with ELF's magic, or one this user can't read, which
//! can't be told apart, is native, its pin keeping the descriptor to run
//! by. Any other is `not_executable`, since `execvp` would hand it to
//! `/bin/sh`.
//!
//! A pin holds a file's identity, and its change time (`ctime`) as it was
//! pinned: a file replaced, removed, or made at its path since is never
//! what the pin runs, and a file rewritten in place, which keeps its
//! identity, is refused at launch, as is one whose mode, owner, or links
//! changed since, since no program of this user's can write a file without
//! moving its change time ([`Pin::check_launch`]). The change time moves by
//! the filesystem clock's tick, so a change within the tick of the file's
//! last change before its pin leaves it as pinned, and isn't seen.
//!
//! A fault is kept as the fault, so a program asked for later meets it
//! then, as the Node module's call-time resolution would.

use std::fs::File;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::fs::{FileExt as _, OpenOptionsExt as _};

use crate::file_id::FileId;

/// A program pinned: what its handle runs.
#[derive(Debug)]
pub struct Pin {
    /// The real path of the file pinned, its handle's `path`.
    pub real: String,
    /// The path it runs at: the real path for a native binary, the path it
    /// was found at for a script.
    pub run_path: String,
    /// Its `argv[0]`: the name for a bare one, else the path it was found
    /// at, absolute.
    pub argv0: String,
    /// The identity of the file pinned.
    pub id: FileId,
    /// Its change time as it was pinned, seconds and nanoseconds.
    pub changed: (i64, i64),
    /// Its link count as it was pinned, which a rename over it or a
    /// removal lowers.
    pub links: u64,
    /// How it runs.
    pub kind: PinKind,
}

impl Pin {
    /// Whether `other` pins the same file the same way, so one entry of a
    /// run's table serves both.
    pub fn same(&self, other: &Self) -> bool {
        self.real == other.real
            && self.run_path == other.run_path
            && self.argv0 == other.argv0
            && self.id == other.id
            && self.changed == other.changed
            && matches!(
                (&self.kind, &other.kind),
                (PinKind::Native(_), PinKind::Native(_)) | (PinKind::Script, PinKind::Script)
            )
    }

    /// That the file the pin runs is still the one pinned, just before
    /// it's launched, `name` the name it was asked for: a native binary's
    /// descriptor, and the path a script was found at, which must still
    /// lead to the pin's identity, each with the change time it was pinned
    /// with. Else the cause of the `launch_failed` the launch throws.
    ///
    /// # Errors
    ///
    /// Returns why it isn't ([`Unlaunched`]): what the launch's `cause`
    /// says, and what the OS said, when it said something.
    pub fn check_launch(&self, name: &str) -> Result<(), Unlaunched> {
        let metadata = match &self.kind {
            // through the held descriptor, so it's the pinned file's
            PinKind::Native(held) => {
                std::fs::metadata(format!("/proc/self/fd/{}", held.as_raw_fd())).map_err(
                    |error| Unlaunched {
                        cause: format!("{name} at {} can't be looked at", self.real),
                        inner: Some(error.to_string()),
                    },
                )?
            }
            PinKind::Script => std::fs::metadata(&self.run_path).map_err(|error| Unlaunched {
                cause: format!(
                    "{name} at {} no longer resolves, pinned to {}",
                    self.run_path, self.real
                ),
                inner: Some(error.to_string()),
            })?,
        };
        if FileId::of(&metadata) != self.id {
            let leads_to = std::fs::canonicalize(&self.run_path)
                .ok()
                .and_then(|real| real.into_os_string().into_string().ok())
                .map_or_else(|| " another file,".to_owned(), |real| format!(" {real},"));
            return Err(Unlaunched::of(format!(
                "{name} at {} now leads to{leads_to} not the file pinned at {}",
                self.run_path, self.real
            )));
        }
        if changed(&metadata) != self.changed {
            // a link gone is the file renamed over or removed, else it was
            // written, or its mode or owner set
            let how = if std::os::unix::fs::MetadataExt::nlink(&metadata) < self.links {
                "was replaced or removed"
            } else {
                "changed"
            };
            return Err(Unlaunched::of(format!(
                "{name} at {} {how} after the run pinned it",
                self.real
            )));
        }
        Ok(())
    }
}

/// The change time of the file `metadata` was read from, seconds and
/// nanoseconds.
fn changed(metadata: &std::fs::Metadata) -> (i64, i64) {
    use std::os::unix::fs::MetadataExt as _;
    (metadata.ctime(), metadata.ctime_nsec())
}

/// Why a pin refused its launch: the cause of the `launch_failed` the
/// launch throws, and what the OS said, that cause's own, when it said
/// something.
#[derive(Debug)]
pub struct Unlaunched {
    /// The cause's message.
    pub cause: String,
    /// What the OS said.
    pub inner: Option<String>,
}

impl Unlaunched {
    /// A refusal of `cause` alone.
    const fn of(cause: String) -> Self {
        Self { cause, inner: None }
    }
}

/// How a pinned program runs.
#[derive(Debug)]
pub enum PinKind {
    /// A native binary, or a file this user can't read, run by this
    /// descriptor on its real path, opened as a path only.
    Native(OwnedFd),
    /// A `#!` script, run by the path it was found at, checked to lead to
    /// the pin's identity just before it's launched.
    Script,
}

/// Why a name pins nothing, which a call asking for it meets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Nothing runnable is at the name.
    NotFound,
    /// What's there can't be run, as this explains.
    NotExecutable(String),
    /// The name leads outside what a name may reach, as this explains.
    NotGranted(String),
    /// The name couldn't be resolved, as `cause` says, a `not_executable`
    /// whose cause's own cause is `why`, what the OS or the run said: a
    /// failure that is no missing file, which is never read as one.
    Unresolved {
        /// The cause's message.
        cause: String,
        /// Why.
        why: String,
    },
}

/// What `name` resolves to and pins: a bare name walked on `path`, the
/// `PATH` children are handed, and a name holding `/` resolved beneath
/// `project`, the project's root, a real path.
///
/// # Errors
///
/// Returns the [`Fault`] a call asking for `name` meets.
pub fn resolve(name: &str, path: Option<&str>, project: &str) -> Result<Pin, Fault> {
    if is_refused_extension(name) {
        return Err(refused_extension(name, name));
    }
    if !name.contains('/') {
        let found = walk_path(name, path)?.ok_or(Fault::NotFound)?;
        return pin(name, &found, name);
    }
    let found = to_project_path(name, project)?;
    // a trailing `/` or `/.` passes through its last component, which only
    // a directory can be
    if name.ends_with('/') || name.ends_with("/.") {
        return Err(Fault::NotFound);
    }
    match probe(&found) {
        Probe::Missing => Err(Fault::NotFound),
        Probe::NotExecutable => Err(Fault::NotExecutable(format!("{found} isn't executable"))),
        Probe::Executable => pin(name, &found, &found),
    }
}

/// Whether `path` ends with a Windows shell's extension, allowing the
/// trailing dots and spaces Windows strips from a file's name.
pub fn is_refused_extension(path: &str) -> bool {
    let trimmed = path.trim_end_matches(['.', ' ']);
    trimmed
        .len()
        .checked_sub(".bat".len())
        .and_then(|at| trimmed.get(at..))
        .is_some_and(|tail| tail.eq_ignore_ascii_case(".bat") || tail.eq_ignore_ascii_case(".cmd"))
}

/// The fault of `name`, found at `path`, a `.bat` or `.cmd` file.
pub fn refused_extension(name: &str, path: &str) -> Fault {
    Fault::NotExecutable(format!(
        "{name} at {path} is a .bat or .cmd file, which only a shell runs"
    ))
}

/// What's at a path as a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    /// A regular file this process may execute.
    Executable,
    /// A regular file it may not.
    NotExecutable,
    /// No regular file: nothing, a path through a file, another kind of
    /// entry, or a stat that failed.
    Missing,
}

/// What's at `path` as a program, symlinks followed.
fn probe(path: &str) -> Probe {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            match gro_sys::may_execute(std::path::Path::new(path)) {
                Ok(true) => Probe::Executable,
                // or gone or changed since the stat, as `access` failing on Node
                Ok(false) | Err(_) => Probe::NotExecutable,
            }
        }
        _ => Probe::Missing,
    }
}

/// The first regular file named `name` this process may execute on
/// `path`'s entries, absolute ones only, each `node_modules/.bin` entry
/// skipped; `None` when no entry has one.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] when an entry has a regular file of
/// the name and none it may execute.
fn walk_path(name: &str, path: Option<&str>) -> Result<Option<String>, Fault> {
    let Some(path) = path else {
        return Ok(None);
    };
    let mut found_not_executable = false;
    for entry in path.split(':') {
        if !entry.starts_with('/') || is_package_bin_entry(entry) {
            continue;
        }
        let candidate = if entry.ends_with('/') {
            format!("{entry}{name}")
        } else {
            format!("{entry}/{name}")
        };
        match probe(&candidate) {
            Probe::Executable => return Ok(Some(candidate)),
            Probe::NotExecutable => found_not_executable = true,
            Probe::Missing => {}
        }
    }
    if found_not_executable {
        return Err(Fault::NotExecutable(format!(
            "{name} is on PATH only without execute permission"
        )));
    }
    Ok(None)
}

/// What a `..` with nothing before it does in [`lexical_segments`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Climb {
    /// Stays at the start, as `/..` is `/`.
    Clamp,
    /// Makes the result `None`.
    Refuse,
}

/// `path`'s segments, resolved by their text: empty and `.` segments
/// dropped, each `..` removing the one before it, and one with none before
/// it as `climb` says.
pub fn lexical_segments(path: &str, climb: Climb) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() && climb == Climb::Refuse {
                    return None;
                }
            }
            segment => segments.push(segment),
        }
    }
    Some(segments)
}

/// Whether a `PATH` entry is a `node_modules/.bin` directory, by its
/// spelling, resolved by its text, no symlink followed.
fn is_package_bin_entry(entry: &str) -> bool {
    let segments = lexical_segments(entry, Climb::Clamp).unwrap_or_default();
    matches!(segments.as_slice(), [.., "node_modules", ".bin"])
}

/// `name`, holding `/`, resolved against `project` by its text.
///
/// # Errors
///
/// Returns [`Fault::NotGranted`] for an absolute name or one climbing above
/// the project.
pub fn to_project_path(name: &str, project: &str) -> Result<String, Fault> {
    if name.starts_with('/') {
        return Err(Fault::NotGranted(format!(
            "{name} is absolute, and a program name with / resolves beneath the project"
        )));
    }
    let Some(segments) = lexical_segments(name, Climb::Refuse) else {
        return Err(Fault::NotGranted(format!(
            "{name} climbs above the project {project}"
        )));
    };
    let prefix = if project == "/" {
        "/".to_owned()
    } else {
        format!("{project}/")
    };
    Ok(prefix + &segments.join("/"))
}

/// The length of the magic a file's first bytes are compared with.
const MAGIC_LENGTH: usize = 4;

/// ELF's magic, the first bytes of a native binary on Linux.
const ELF_MAGIC: [u8; MAGIC_LENGTH] = [0x7f, b'E', b'L', b'F'];

/// Pins `found`, the file `name` was found at, whose `argv[0]` is `argv0`.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] for a real path that isn't UTF-8 or
/// can't be read, a `.bat` or `.cmd` pin, a file that can't be opened or
/// read but for a denied read, and one whose first bytes are neither `#!`
/// nor ELF's magic.
fn pin(name: &str, found: &str, argv0: &str) -> Result<Pin, Fault> {
    let no_real_path = || {
        Fault::NotExecutable(format!(
            "{name} at {found} has no real path a string can name"
        ))
    };
    let real = std::fs::canonicalize(found)
        .map_err(|_| no_real_path())?
        .into_os_string()
        .into_string()
        .map_err(|_| no_real_path())?;
    if is_refused_extension(&real) {
        return Err(refused_extension(name, &real));
    }
    let held: OwnedFd = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(&real)
        .map_err(|_| Fault::NotExecutable(format!("{name} at {real} can't be read")))?
        .into();
    pin_held(name, held, real, found, argv0)
}

/// Pins the file `held` names, opened as a path only, found at `found`,
/// whose real path is `real` and whose `argv[0]` is `argv0`: what a pin of
/// a found path does once it's opened, and what `program_in` does with the
/// file it opened beneath its directory.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] for a file that can't be read but for
/// a denied read, and one whose first bytes are neither `#!` nor ELF's
/// magic.
pub fn pin_held(
    name: &str,
    held: OwnedFd,
    real: String,
    found: &str,
    argv0: &str,
) -> Result<Pin, Fault> {
    let unreadable = || Fault::NotExecutable(format!("{name} at {real} can't be read"));
    let held = File::from(held);
    let metadata = held.metadata().map_err(|_| unreadable())?;
    let id = FileId::of(&metadata);
    // reopened through the held descriptor, so the bytes read are the
    // pinned file's whatever its path leads to by now; non-blocking, so a
    // FIFO can't hang the open
    let reading = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(format!("/proc/self/fd/{}", held.as_raw_fd()));
    let script = match reading {
        Ok(reading) => {
            // zeroed, so a file shorter than the magic can't match it
            let mut magic = [0; MAGIC_LENGTH];
            reading.read_at(&mut magic, 0).map_err(|_| unreadable())?;
            if magic.starts_with(b"#!") {
                true
            } else if magic == ELF_MAGIC {
                false
            } else {
                return Err(Fault::NotExecutable(format!(
                    "{name} at {real} starts with neither #! nor a native binary's magic"
                )));
            }
        }
        // a file this user can't read can't be inspected, and no
        // interpreter could read it as a script
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(_) => return Err(unreadable()),
    };
    Ok(Pin {
        run_path: if script {
            found.to_owned()
        } else {
            real.clone()
        },
        real,
        argv0: argv0.to_owned(),
        id,
        changed: changed(&metadata),
        links: std::os::unix::fs::MetadataExt::nlink(&metadata),
        kind: if script {
            PinKind::Script
        } else {
            PinKind::Native(held.into())
        },
    })
}

#[cfg(test)]
mod tests;
