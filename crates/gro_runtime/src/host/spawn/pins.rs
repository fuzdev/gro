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
//! A script found at a name spelled as a `node_modules/.bin` program
//! (`node_modules/.bin/<x>`, its last three segments, once resolved by its
//! text, `node_modules`, `.bin`, and the program) runs under `node` alone,
//! never by its path: `npm run` puts each `node_modules/.bin` up from the
//! project on a child's `PATH`, absolute, ahead of the rest, so the
//! `/usr/bin/env node` such a script names would find a `node` a package
//! installed there, or the run wrote there, at each exec. Its `#!` line,
//! read through the pinned descriptor, is `#!/usr/bin/env node` or
//! `#!/usr/bin/env -S node` and its flags, each a plain word starting with
//! `-`, or the name is `not_executable`, naming the line; `node` is found
//! by the walk a bare name takes and pinned beside the script, a `node`
//! that is itself a script (a version manager's shim) pinned as one, its
//! own line the kernel's to read on the child's `PATH`, so a shim's
//! interpreter is found where a package can plant one; and the launch runs
//! that `node`, as `node`, with the flags, then the script's real path,
//! then the call's arguments ([`PinKind::NodeScript`]). A script reached by
//! another name, a bare one on `PATH` included, runs by its path, its
//! interpreter the kernel's to find at each exec.
//!
//! Kind: a boundary for which `node` runs, a native check before the
//! launch, when the `node` found is the runtime itself. A `node` that finds
//! what it runs on the child's `PATH`, a script shim or a native launcher,
//! defeats it at no more effort than the threat it closes, a `bash` or
//! `node` linked into a `node_modules/.bin`; and the script, install-time
//! code, is opened by `node` at its real path after the launch check, so
//! code that can write a directory on that path, the run's own with a write
//! grant there, swaps what runs.
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
use crate::host::fs::reads::escaped;

/// A program pinned: what its handle runs.
#[derive(Debug)]
pub struct Pin {
    /// The real path of the file pinned, its handle's `path`.
    pub real: String,
    /// The path it runs at: the real path for a native binary, the path it
    /// was found at for a script, and the real path, handed to its `node`,
    /// for a `node_modules/.bin` script.
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
            && match (&self.kind, &other.kind) {
                (PinKind::Native(_), PinKind::Native(_)) | (PinKind::Script, PinKind::Script) => {
                    true
                }
                (
                    PinKind::NodeScript { node, flags },
                    PinKind::NodeScript {
                        node: other_node,
                        flags: other_flags,
                    },
                ) => node.same(other_node) && flags == other_flags,
                _ => false,
            }
    }

    /// That the file the pin runs is still the one pinned, just before
    /// it's launched, `name` the name it was asked for: a native binary's
    /// descriptor, the path a script was found at, and a
    /// `node_modules/.bin` script's real path, which must still lead to the
    /// pin's identity, each with the change time it was pinned with, and
    /// then the `node` such a script runs under, as its own pin. Else the
    /// cause of the `launch_failed` the launch throws.
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
            PinKind::Script | PinKind::NodeScript { .. } => std::fs::metadata(&self.run_path)
                .map_err(|error| Unlaunched {
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
                .and_then(|real| real.into_os_string().into_string().ok());
            // another file at the pinned real path, renamed over it, is told
            // apart by what it is, not where it is
            return Err(Unlaunched::of(match leads_to {
                Some(real) if real == self.real => {
                    format!("{name} at {} is no longer the file pinned", self.run_path)
                }
                Some(real) => format!(
                    "{name} at {} now leads to {real}, not the file pinned at {}",
                    self.run_path, self.real
                ),
                None => format!(
                    "{name} at {} now leads to another file, not the file pinned at {}",
                    self.run_path, self.real
                ),
            }));
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
        if let PinKind::NodeScript { node, .. } = &self.kind {
            node.check_launch(NODE)?;
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
    /// A `node_modules/.bin` script, run by its `node`, pinned beside it,
    /// as `node <flags> <real path>`, its real path checked to lead to the
    /// pin's identity just before it's launched, and its `node` checked as
    /// its own pin.
    NodeScript {
        /// The `node` it runs under: native, or a script the kernel reads,
        /// never a `node_modules/.bin` script itself.
        node: Box<Pin>,
        /// The flags its `#!/usr/bin/env -S node` line passes, in order.
        flags: Vec<String>,
    },
}

/// The interpreter a `node_modules/.bin` script runs under, by its name on
/// `PATH`, and its own `argv[0]`.
const NODE: &str = "node";

/// How a `#!` script a pin finds runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptRun<'a> {
    /// By the path it was found at, the kernel reading its line at each
    /// exec.
    ByPath,
    /// Under `node`, found on this `PATH`, its line read and checked as
    /// it's pinned ([`PinKind::NodeScript`]).
    UnderNode(Option<&'a str>),
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
        return pin(name, &found, name, ScriptRun::ByPath);
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
        Probe::Executable => {
            let run = if is_package_bin_name(name) {
                ScriptRun::UnderNode(path)
            } else {
                ScriptRun::ByPath
            };
            pin(name, &found, &found, run)
        }
    }
}

/// Whether a program's name, holding `/`, names a `node_modules/.bin`
/// program, by its spelling, resolved by its text, no symlink followed:
/// its last three segments `node_modules`, `.bin`, and the program.
pub fn is_package_bin_name(name: &str) -> bool {
    let segments = lexical_segments(name, Climb::Clamp).unwrap_or_default();
    matches!(segments.as_slice(), [.., "node_modules", ".bin", _])
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

/// Pins `found`, the file `name` was found at, whose `argv[0]` is `argv0`,
/// a script running as `run` says.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] for a real path that isn't UTF-8 or
/// can't be read, a `.bat` or `.cmd` pin, a file that can't be opened or
/// read but for a denied read, and one whose first bytes are neither `#!`
/// nor ELF's magic, and [`pin_held`]'s faults for a script run under
/// `node`.
fn pin(name: &str, found: &str, argv0: &str, run: ScriptRun<'_>) -> Result<Pin, Fault> {
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
    pin_held(name, held, real, found, argv0, run)
}

/// Pins the file `held` names, opened as a path only, found at `found`,
/// whose real path is `real` and whose `argv[0]` is `argv0`, a script
/// running as `run` says: what a pin of a found path does once it's
/// opened, and what `program_in` does with the file it opened beneath its
/// directory.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] for a file that can't be read but for
/// a denied read, and one whose first bytes are neither `#!` nor ELF's
/// magic; for a script run under `node`, one whose `#!` line is no `node`
/// line ([`node_flags`]) or no `node` on `PATH`, and a `node` that pins
/// nothing, [`Fault::Unresolved`], its fault as the cause's own.
pub fn pin_held(
    name: &str,
    held: OwnedFd,
    real: String,
    found: &str,
    argv0: &str,
    run: ScriptRun<'_>,
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
    let kind = match reading {
        Ok(reading) => {
            // zeroed, so a file shorter than the magic can't match it
            let mut magic = [0; MAGIC_LENGTH];
            reading.read_at(&mut magic, 0).map_err(|_| unreadable())?;
            if !magic.starts_with(b"#!") && magic != ELF_MAGIC {
                return Err(Fault::NotExecutable(format!(
                    "{name} at {real} starts with neither #! nor a native binary's magic"
                )));
            }
            match run {
                _ if magic == ELF_MAGIC => PinKind::Native(held.into()),
                ScriptRun::ByPath => PinKind::Script,
                ScriptRun::UnderNode(path) => {
                    let (line, whole) = read_line(&reading).map_err(|_| unreadable())?;
                    let flags = whole.then(|| node_flags(&line)).flatten().ok_or_else(|| {
                        Fault::NotExecutable(format!(
                            "{name} at {real} starts with `{}`, and a node_modules/.bin script \
                             runs pinned only under node, by `#!/usr/bin/env node` or \
                             `#!/usr/bin/env -S node` and its flags",
                            escaped(&line)
                        ))
                    })?;
                    let node = pin_node(name, &real, path)?;
                    PinKind::NodeScript {
                        node: Box::new(node),
                        flags,
                    }
                }
            }
        }
        // a file this user can't read can't be inspected, and no
        // interpreter could read it as a script
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            PinKind::Native(held.into())
        }
        Err(_) => return Err(unreadable()),
    };
    Ok(Pin {
        run_path: match kind {
            PinKind::Native(_) | PinKind::NodeScript { .. } => real.clone(),
            PinKind::Script => found.to_owned(),
        },
        real,
        argv0: argv0.to_owned(),
        id,
        changed: changed(&metadata),
        links: std::os::unix::fs::MetadataExt::nlink(&metadata),
        kind,
    })
}

/// The most of a file's first line the kernel reads for its `#!`.
const LINE_MAX: usize = 256;

/// The first line of the file `reading` reads, from its first byte, without
/// its newline, and whether it's whole: up to the first newline in its
/// first [`LINE_MAX`] bytes, or the whole file when it's shorter and holds
/// none; else those bytes, not whole, which no `node` line is.
///
/// # Errors
///
/// Returns the read's error.
fn read_line(reading: &File) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = vec![0; LINE_MAX];
    let mut read = 0;
    while read < LINE_MAX {
        let more = reading.read_at(&mut bytes[read..], u64::try_from(read).unwrap_or(u64::MAX))?;
        if more == 0 {
            break;
        }
        read += more;
    }
    bytes.truncate(read);
    match bytes.iter().position(|&byte| byte == b'\n') {
        Some(end) => {
            bytes.truncate(end);
            Ok((bytes, true))
        }
        None => Ok((bytes, read < LINE_MAX)),
    }
}

/// The flags a `node_modules/.bin` script's first line, `line`, passes to
/// `node`: none for `#!/usr/bin/env node`, and those after `node` for
/// `#!/usr/bin/env -S node <flags>`, each a word of letters, digits, and
/// `-_=.,:/+@%` starting with `-`, so `env -S` would split and pass it as
/// written; the words split at spaces and tabs, as the kernel and `env -S`
/// split them. `None` for any other line: another interpreter, a `node`
/// with words but no `-S`, which `env` would take for one program's name,
/// a word `env -S` would expand, unquote, or end at, or anything not
/// printable ASCII, a carriage return among it.
fn node_flags(line: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(line.strip_prefix(b"#!")?).ok()?;
    if !text
        .bytes()
        .all(|byte| byte == b'\t' || (0x20..0x7f).contains(&byte))
    {
        return None;
    }
    let words: Vec<&str> = text
        .split([' ', '\t'])
        .filter(|word| !word.is_empty())
        .collect();
    let flags = match words.as_slice() {
        ["/usr/bin/env", "node"] => &[][..],
        ["/usr/bin/env", "-S", "node", flags @ ..] => flags,
        _ => return None,
    };
    let plain = |flag: &&str| {
        flag.starts_with('-')
            && flag
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_=.,:/+@%".contains(&byte))
    };
    flags
        .iter()
        .all(plain)
        .then(|| flags.iter().map(|&flag| flag.to_owned()).collect())
}

/// `node`, found by the walk a bare name takes on `path` and pinned, for
/// `name`, a `node_modules/.bin` script at `real`; a `node` that is itself
/// a script pins as one, by its path.
///
/// # Errors
///
/// Returns [`Fault::NotExecutable`] when no entry of `path` holds a
/// `node`, and [`Fault::Unresolved`], `node`'s own fault as the cause's,
/// when what's found pins nothing.
fn pin_node(name: &str, real: &str, path: Option<&str>) -> Result<Pin, Fault> {
    let unpinned = |fault: Fault| {
        let why = match fault {
            Fault::NotFound => "nothing is there".to_owned(),
            Fault::NotExecutable(why) | Fault::NotGranted(why) => why,
            Fault::Unresolved { cause, why } => format!("{cause}: {why}"),
        };
        Fault::Unresolved {
            cause: format!("{name} at {real} runs under node, which can't be pinned"),
            why,
        }
    };
    let found = walk_path(NODE, path).map_err(unpinned)?.ok_or_else(|| {
        Fault::NotExecutable(format!(
            "{name} at {real} runs under node, and no PATH entry outside a node_modules/.bin \
             holds one"
        ))
    })?;
    pin(NODE, &found, NODE, ScriptRun::ByPath).map_err(unpinned)
}

#[cfg(test)]
mod tests;
