//! What git reads that the box's facts grant: its home config, the files it
//! includes and its `core.excludesFile` and `core.attributesFile`, and the
//! project's own git directories.
//!
//! **Home config.** With `GIT_CONFIG_GLOBAL` unset, git reads
//! `~/.gitconfig` and `config` in its directory under `XDG_CONFIG_HOME`
//! (or `~/.config`); with it set, the file it names alone. Beside those,
//! git reads `ignore` and `attributes` in that directory, the defaults of
//! `core.excludesFile` and `core.attributesFile`. Each config file is read
//! here as git reads it (the grammar below), following `include.path` and
//! `includeIf.<condition>.path` (whatever the condition, since a read rule
//! costs nothing where git doesn't read) to `MAX_INCLUDE_DEPTH`, and taking
//! `core.excludesFile` and `core.attributesFile`, each path as git resolves
//! it: `~/` from the home directory, an include's relative path from the
//! directory of the including file as git named it (a symlink's, not its
//! target's), and `GIT_CONFIG_GLOBAL` and the other relative paths from the
//! repository's top, where git runs from.
//!
//! What exists of each is granted by its real path, but nothing in the
//! repository's tree or reached through it, whose contents the project
//! writes: a symlink the project plants where a config points, or where a
//! link of the user's into the tree leads, is no rule outside it, and a
//! config there isn't followed. A target that is a directory, an include
//! nested past git's limit, or a config the reader can't read is a
//! [`GitFault`], which refuses a run that may start a program. Each file is
//! read only when it's a regular file, and at most `MAX_READ` bytes, so
//! nothing planted (a FIFO, `/dev/zero`) is waited on or read whole.
//!
//! **The grammar** is git's (`config.c`, held to git 2.47 by hand): bytes,
//! a leading BOM skipped, `\r\n` read as `\n`, whitespace git's own (a
//! space, a tab, a lone `\r`, never a form feed); a `#` or `;` comment to
//! the line's end; a section header, `[name]` (its name letters, digits,
//! `-`, and `.`, a dotted one the legacy `[section.subsection]`, all
//! lowercased) or `[name "subsection"]` (the name possibly empty, any byte
//! escaped by `\`), whose line goes on with a variable; a variable, a letter
//! then letters, digits, and `-`, then spaces or tabs, with no `=` (a
//! boolean) or `= value`, the value's unquoted whitespace kept byte for byte
//! between words and dropped at its ends, quoted runs kept whole,
//! `\\`, `\"`, `\n`, `\t`, `\b` read, `\` before the line's end continuing
//! it, and an unquoted `#` or `;` ending it. Refused, as git refuses each:
//! a header unclosed, with nothing in it, or with a byte outside its name's
//! set; a variable not starting with a letter or followed by anything but
//! `=` or the line's end; an unknown escape; a quote open at the line's end.
//! Beyond git, a `~user/` or `%(prefix)/` path, whose home or prefix the
//! box can't tell, is refused.
//!
//! **Git directories.** The project's `.git`, or where there's none at the
//! root the nearest ancestor's, as git discovers it: a directory is read
//! where it is (at the root the project's rule covers it already). A file
//! (`gitdir: <path>`) is accepted as one of two layouts, each checked to
//! lead back to this checkout, else a [`GitFault`]:
//!
//! - a linked worktree's: its git directory is `<common>/worktrees/<name>`,
//!   its `commondir` leads to `<common>`, `<common>` is a directory named
//!   `.git` or a bare repository (`HEAD`, `objects`, `refs`), and its
//!   `gitdir` file, relative to the git directory where it's relative,
//!   leads back to this `.git` file; the git directory and `<common>` are
//!   read;
//! - a submodule's: its git directory is a git directory (`HEAD`,
//!   `objects`, `refs`) beneath `<super>/.git/modules/`, or beneath a
//!   linked worktree's `<super>/.git/worktrees/<w>/modules/` (nested
//!   modules too), and its `config`'s `core.worktree`, relative to the git
//!   directory, leads back to this checkout; the git directory is read.
//!
//! Each of these small files is read with every `\r` and `\n` at its end
//! taken off, as git reads them.
//!
//! Anything else (a `--separate-git-dir` one, one naming a directory that
//! isn't a git directory, like `~/.ssh`, or another checkout's worktree or
//! submodule) is a fault. An ancestor's `.git` at the home directory or
//! above, a dotfiles repository's, gets no rule, and git run there fails as
//! where it can't read one; the home directory is compared by its real
//! path.
//!
//! Kind: these are facts the box's rules are derived from, the read rules
//! part of its boundary. The adversary of the git directories is the
//! project itself, which controls a root `.git` file and everything in the
//! repository's tree; the backlinks hold its `gitdir:` to this checkout's
//! own worktree or submodule, and the tree check keeps its links from
//! becoming rules. The home config isn't the project's to write, so its
//! includes outside the tree are the user's.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

/// How deep includes of includes are followed, as git's own limit.
const MAX_INCLUDE_DEPTH: usize = 10;

/// The most bytes read of a config or of a git directory's file, so a
/// planted link to `/dev/zero` can't exhaust memory before the box.
const MAX_READ: u64 = 1 << 20;

/// What git reads in its home config directory beside `config`, the
/// defaults of `core.excludesFile` and `core.attributesFile`.
const GIT_HOME_DEFAULTS: [&str; 2] = ["ignore", "attributes"];

/// `/dev/null`'s device number, a character device's, which git reads as an
/// empty file.
const DEV_NULL: u64 = libc::makedev(1, 3);

/// The UTF-8 byte order mark git skips at a config's start.
const BOM: &[u8] = b"\xef\xbb\xbf";

/// Why git's config or directories can't be read for the box.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {why}", path.display())]
pub struct GitFault {
    /// The file the fault is in.
    pub path: PathBuf,
    /// What's wrong, with the line where there is one.
    pub why: String,
}

/// The process inputs and facts git's home reads depend on.
#[derive(Debug, Clone, Copy)]
pub(super) struct GitHomeInputs<'a> {
    /// The repository's top, where git runs from: the directory holding the
    /// `.git` found, else the project's root.
    pub top: &'a Path,
    /// `HOME`, absolute, if it's set.
    pub home: Option<&'a Path>,
    /// `XDG_CONFIG_HOME`, absolute, if it's set.
    pub xdg_config_home: Option<&'a Path>,
    /// `GIT_CONFIG_GLOBAL`, if it's set.
    pub config_global: Option<&'a Path>,
}

/// The real paths of the files git reads from its home config, each that
/// exists, in the order met, each once.
///
/// # Errors
///
/// Returns the first [`GitFault`]: a config the reader refuses, an
/// include beyond `MAX_INCLUDE_DEPTH`, or a named path that is a directory.
pub(super) fn home_files(home: &GitHomeInputs<'_>) -> Result<Vec<PathBuf>, GitFault> {
    let config_home = home
        .xdg_config_home
        .map(Path::to_owned)
        .or_else(|| home.home.map(|home| home.join(".config")))
        .map(|dir| dir.join("git"));
    let configs: Vec<PathBuf> = home.config_global.map_or_else(
        || {
            home.home
                .map(|home| home.join(".gitconfig"))
                .into_iter()
                .chain(config_home.iter().map(|dir| dir.join("config")))
                .collect()
        },
        |global| vec![home.top.join(global)],
    );
    let mut found = Found {
        top: home.top,
        files: Vec::new(),
    };
    for config in &configs {
        read_config(config, home, 0, &mut found)?;
    }
    for name in GIT_HOME_DEFAULTS {
        if let Some(dir) = &config_home {
            found.add_file(&dir.join(name))?;
        }
    }
    Ok(found.files)
}

/// The real paths read so far, each once.
#[derive(Debug)]
struct Found<'a> {
    /// The repository's top, whose tree gets no rule of its own.
    top: &'a Path,
    /// The files, in the order met.
    files: Vec<PathBuf>,
}

impl Found<'_> {
    /// Adds what's at `path`, by its real path, when it resolves: `Some`
    /// of the real path when it's new, `None` when it's missing, met
    /// already, or in the repository's tree or reached through it.
    ///
    /// # Errors
    ///
    /// Returns a [`GitFault`] when it's a directory.
    fn add_file(&mut self, path: &Path) -> Result<Option<PathBuf>, GitFault> {
        let Ok(real) = std::fs::canonicalize(path) else {
            return Ok(None);
        };
        if meets_tree(path, self.top) {
            return Ok(None);
        }
        if real.is_dir() {
            return Err(GitFault {
                path: path.to_owned(),
                why: "names a directory, and the box grants no directory from git's config"
                    .to_owned(),
            });
        }
        if self.files.contains(&real) {
            return Ok(None);
        }
        self.files.push(real.clone());
        Ok(Some(real))
    }
}

/// Whether resolving `path` meets the repository's tree at `top`, a real
/// path: at a directory it passes through, a link it follows, or what it
/// leads to, each where the walk stands, so a link outside the tree that
/// leads into it and back out is met as well as one inside it.
///
/// The walk resolves one component at a time as the kernel does, each link
/// in place, so where it stands is always a real path; one past the
/// kernel's link limit, or one whose link can't be read, counts as meeting
/// it, granting nothing.
fn meets_tree(path: &Path, top: &Path) -> bool {
    use std::path::Component;
    /// The kernel's limit on links followed in one resolution.
    const MAX_LINKS: usize = 40;
    // the components still to walk, the next last
    let mut rest: Vec<PathBuf> = path
        .components()
        .rev()
        .map(|part| PathBuf::from(part.as_os_str()))
        .collect();
    let mut at = PathBuf::from("/");
    let mut links = 0;
    while let Some(part) = rest.pop() {
        match part.components().next() {
            Some(Component::RootDir) => at = PathBuf::from("/"),
            Some(Component::ParentDir) => {
                at.pop();
            }
            Some(Component::Normal(name)) => {
                let next = at.join(name);
                match std::fs::symlink_metadata(&next) {
                    Ok(metadata) if metadata.is_symlink() => {
                        links += 1;
                        let Ok(target) = std::fs::read_link(&next) else {
                            return true;
                        };
                        if links > MAX_LINKS {
                            return true;
                        }
                        rest.extend(
                            target
                                .components()
                                .rev()
                                .map(|part| PathBuf::from(part.as_os_str())),
                        );
                    }
                    _ => at = next,
                }
            }
            Some(Component::CurDir | Component::Prefix(_)) | None => {}
        }
        if at.starts_with(top) {
            return true;
        }
    }
    false
}

/// Reads the config at `path`, `depth` includes deep, adding it and what
/// it names to `found`.
fn read_config(
    path: &Path,
    home: &GitHomeInputs<'_>,
    depth: usize,
    found: &mut Found<'_>,
) -> Result<(), GitFault> {
    let Some(real) = found.add_file(path)? else {
        return Ok(());
    };
    let fault = |why: String| GitFault {
        path: real.clone(),
        why,
    };
    let bytes = read_small(&real).map_err(|why| fault(why.to_owned()))?;
    // git resolves an include from the file as it named it, a symlink's
    // directory, not its target's
    let base = path.parent().unwrap_or_else(|| Path::new("/"));
    for (key, value) in parse_config(&bytes).map_err(fault)? {
        let named = match key {
            Key::Include => {
                let target = resolve(&value, home.home, base).map_err(fault)?;
                // git counts an include toward its limit only where the
                // file is there
                if depth + 1 > MAX_INCLUDE_DEPTH {
                    if target.exists() {
                        return Err(fault(format!(
                            "includes nest deeper than {MAX_INCLUDE_DEPTH}, as git refuses too"
                        )));
                    }
                    continue;
                }
                read_config(&target, home, depth + 1, found)?;
                continue;
            }
            Key::File => resolve(&value, home.home, home.top).map_err(fault)?,
            Key::Worktree => continue,
        };
        found.add_file(&named)?;
    }
    Ok(())
}

/// The bytes of the regular file at `path`, at most `MAX_READ` of them; a
/// FIFO, a device, or a longer file is refused, never waited on or read
/// whole.
///
/// # Errors
///
/// Returns why, as a fault's text.
fn read_small(path: &Path) -> Result<Vec<u8>, &'static str> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    // held to a regular file before it's opened, as a device's open acts
    // (a serial line's raises its modem lines), then opened without waiting
    // (a FIFO's open would) and held to one again by what was opened; but
    // `/dev/null`, which git reads as empty (`GIT_CONFIG_GLOBAL=/dev/null`),
    // is empty, never opened
    let Ok(stated) = std::fs::metadata(path) else {
        return Err("can't be opened");
    };
    if stated.file_type().is_char_device() && stated.rdev() == DEV_NULL {
        return Ok(Vec::new());
    }
    if !stated.is_file() {
        return Err("isn't a regular file");
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(|_| "can't be opened")?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err("isn't a regular file");
    }
    let mut bytes = Vec::new();
    file.take(MAX_READ + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "can't be read")?;
    if bytes.len() as u64 > MAX_READ {
        return Err("is longer than the box's reader reads, 1 MiB");
    }
    Ok(bytes)
}

/// A key the reader acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// `include.path` or `includeIf.<condition>.path`.
    Include,
    /// `core.excludesFile` or `core.attributesFile`.
    File,
    /// `core.worktree`.
    Worktree,
}

/// The keys of `bytes`, a git config, the reader acts on, with their
/// values, in order.
///
/// # Errors
///
/// Returns what the [module doc](self)'s grammar refuses, with its line.
fn parse_config(bytes: &[u8]) -> Result<Vec<(Key, Vec<u8>)>, String> {
    let mut source = Source {
        bytes: bytes.strip_prefix(BOM).unwrap_or(bytes),
        at: 0,
        line: 1,
        read_line: 1,
    };
    let mut keys = Vec::new();
    // the section's name, lowercased, and whether it has a subsection
    let mut section: (Vec<u8>, bool) = (Vec::new(), false);
    let mut comment = false;
    loop {
        let Some(byte) = source.next() else {
            return Ok(keys);
        };
        if byte == b'\n' {
            comment = false;
            continue;
        }
        if comment || is_space(byte) {
            continue;
        }
        match byte {
            b'#' | b';' => comment = true,
            b'[' => section = source.header()?,
            byte if byte.is_ascii_alphabetic() => {
                let (name, value) = source.variable(byte)?;
                let key = match (section.0.as_slice(), section.1, name.as_slice()) {
                    (b"include", false, b"path") | (b"includeif", true, b"path") => Key::Include,
                    (b"core", false, b"excludesfile" | b"attributesfile") => Key::File,
                    (b"core", false, b"worktree") => Key::Worktree,
                    _ => continue,
                };
                // a variable with no `=` is a boolean, which names no path
                if let Some(value) = value {
                    keys.push((key, value));
                }
            }
            _ => return Err(source.bad()),
        }
    }
}

/// A config's bytes as git reads them, one at a time.
struct Source<'a> {
    /// The bytes.
    bytes: &'a [u8],
    /// The next byte's place.
    at: usize,
    /// The line the next byte is on.
    line: usize,
    /// The line the last byte read is on, a line's end on its own line.
    read_line: usize,
}

impl Source<'_> {
    /// The next byte, `\r\n` read as `\n`; `None` at the end.
    fn next(&mut self) -> Option<u8> {
        let mut byte = *self.bytes.get(self.at)?;
        self.at += 1;
        self.read_line = self.line;
        if byte == b'\r' && self.bytes.get(self.at) == Some(&b'\n') {
            self.at += 1;
            byte = b'\n';
        }
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    /// The next byte, the line's end at the end, as git reads it.
    fn next_or_end(&mut self) -> u8 {
        self.next().unwrap_or(b'\n')
    }

    /// The refusal of the line the reader is on, as git words its own.
    fn bad(&self) -> String {
        format!("bad config line {}", self.read_line)
    }

    /// A section header after its `[`, the line going on after its `]`:
    /// its name lowercased, and whether it has a subsection.
    fn header(&mut self) -> Result<(Vec<u8>, bool), String> {
        let mut name = Vec::new();
        loop {
            match self.next() {
                Some(b']') if !name.is_empty() => {
                    return Ok(match name.iter().position(|&byte| byte == b'.') {
                        Some(_) => (
                            name.split(|&byte| byte == b'.')
                                .next()
                                .unwrap_or_default()
                                .to_vec(),
                            true,
                        ),
                        None => (name, false),
                    });
                }
                Some(byte) if is_space(byte) && byte != b'\n' => {
                    return self.subsection(name);
                }
                Some(byte) if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.' => {
                    name.push(byte.to_ascii_lowercase());
                }
                _ => return Err(self.bad()),
            }
        }
    }

    /// A header's quoted subsection after its name and a space, through
    /// the header's `]`.
    fn subsection(&mut self, name: Vec<u8>) -> Result<(Vec<u8>, bool), String> {
        let mut byte = self.next_or_end();
        while is_space(byte) && byte != b'\n' {
            byte = self.next_or_end();
        }
        if byte != b'"' {
            return Err(self.bad());
        }
        loop {
            match self.next_or_end() {
                b'\n' => return Err(self.bad()),
                b'"' => break,
                b'\\' if self.next_or_end() == b'\n' => return Err(self.bad()),
                _ => {}
            }
        }
        if self.next_or_end() == b']' {
            Ok((name, true))
        } else {
            Err(self.bad())
        }
    }

    /// A variable whose name starts with `first`, through its line's end:
    /// its name lowercased, and its value, `None` for a boolean.
    fn variable(&mut self, first: u8) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
        let mut name = vec![first.to_ascii_lowercase()];
        let mut byte = self.next_or_end();
        while byte.is_ascii_alphanumeric() || byte == b'-' {
            name.push(byte.to_ascii_lowercase());
            byte = self.next_or_end();
        }
        while byte == b' ' || byte == b'\t' {
            byte = self.next_or_end();
        }
        match byte {
            b'\n' => Ok((name, None)),
            b'=' => Ok((name, Some(self.value()?))),
            _ => Err(self.bad()),
        }
    }

    /// A value after its `=`, through its line's end, as git's
    /// `parse_value` reads it.
    fn value(&mut self) -> Result<Vec<u8>, String> {
        let (mut value, mut quoted, mut comment, mut spaces) =
            (Vec::new(), false, false, Vec::new());
        loop {
            let byte = self.next_or_end();
            if byte == b'\n' {
                return if quoted { Err(self.bad()) } else { Ok(value) };
            }
            if comment {
                continue;
            }
            if is_space(byte) && !quoted {
                if !value.is_empty() {
                    spaces.push(byte);
                }
                continue;
            }
            if !quoted && (byte == b'#' || byte == b';') {
                comment = true;
                continue;
            }
            value.append(&mut spaces);
            match byte {
                b'\\' => match self.next_or_end() {
                    b'\n' => {}
                    b't' => value.push(b'\t'),
                    b'b' => value.push(8),
                    b'n' => value.push(b'\n'),
                    escaped @ (b'\\' | b'"') => value.push(escaped),
                    _ => return Err(self.bad()),
                },
                b'"' => quoted = !quoted,
                byte => value.push(byte),
            }
        }
    }
}

/// Whether `byte` is whitespace as git's `isspace` reads it: a space, a
/// tab, or a line's end, never a form feed or a vertical tab.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// A path value as git resolves it: `~` and `~/` from `home`, absolute as
/// it is, relative from `base`.
fn resolve(value: &[u8], home: Option<&Path>, base: &Path) -> Result<PathBuf, String> {
    let shown = String::from_utf8_lossy(value);
    if value.starts_with(b"%(prefix)/") {
        return Err(format!("{shown:?} names a path from git's install prefix"));
    }
    let home_relative = if value == b"~" {
        Some(&b""[..])
    } else {
        value.strip_prefix(b"~/")
    };
    if let Some(rest) = home_relative {
        return home
            .map(|home| home.join(OsStr::from_bytes(rest)))
            .ok_or_else(|| format!("{shown:?} names the home directory, and HOME isn't absolute"));
    }
    if value.starts_with(b"~") {
        return Err(format!("{shown:?} names another user's home directory"));
    }
    Ok(base.join(OsStr::from_bytes(value)))
}

/// The project's git directories outside its root that git reads, and the
/// repository's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitDirs {
    /// Each by its real path: an ancestor's `.git`, a worktree's git
    /// directory and its common directory, or a submodule's git directory.
    pub dirs: Vec<PathBuf>,
    /// The directory holding the `.git` found, else the project's root.
    pub top: PathBuf,
}

/// The project's git directories outside its root that git reads.
///
/// None when the root's `.git` is a directory, or no `.git` is found below
/// `home`, compared by its real path.
///
/// # Errors
///
/// Returns a [`GitFault`] for a `.git` file the [module doc](self) doesn't
/// accept.
pub(super) fn git_dirs(root: &Path, home: Option<&Path>) -> Result<GitDirs, GitFault> {
    // the root is a real path, so the home directory is compared as one
    let home = home.map(|home| std::fs::canonicalize(home).unwrap_or_else(|_| home.to_owned()));
    let none = || GitDirs {
        dirs: Vec::new(),
        top: root.to_owned(),
    };
    for dir in root.ancestors() {
        if home.as_deref().is_some_and(|home| home.starts_with(dir)) {
            return Ok(none());
        }
        let dot_git = dir.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        let dirs = if metadata.is_dir() {
            if dir == root {
                Vec::new()
            } else {
                vec![dot_git]
            }
        } else if metadata.is_file() {
            linked_dirs(&dot_git)?
        } else {
            return Err(GitFault {
                path: dot_git,
                why: "is neither a directory nor a file".to_owned(),
            });
        };
        return Ok(GitDirs {
            dirs,
            top: dir.to_owned(),
        });
    }
    Ok(none())
}

/// The git directories of the `.git` file at `dot_git`, a linked
/// worktree's or a submodule's, checked as the [module doc](self) says.
fn linked_dirs(dot_git: &Path) -> Result<Vec<PathBuf>, GitFault> {
    let fault = |why: &str| GitFault {
        path: dot_git.to_owned(),
        why: why.to_owned(),
    };
    let text = read_small(dot_git).map_err(fault)?;
    let named = trim_line_ends(&text)
        .strip_prefix(b"gitdir: ")
        .filter(|named| !named.is_empty() && !named.contains(&b'\n'))
        .ok_or_else(|| fault("holds no single gitdir: line"))?;
    let holder = dot_git.parent().unwrap_or_else(|| Path::new("/"));
    let git_dir = std::fs::canonicalize(holder.join(OsStr::from_bytes(named)))
        .map_err(|_| fault("names a git directory that doesn't resolve"))?;
    let commondir = git_dir.join("commondir");
    match read_small(&commondir) {
        Ok(text) => worktree_dirs(dot_git, git_dir, &text).map_err(fault),
        // there, but not a file the reader reads (a planted FIFO, a device)
        Err(why) if std::fs::symlink_metadata(&commondir).is_ok() => Err(GitFault {
            path: commondir,
            why: format!("names a worktree's commondir that {why}"),
        }),
        Err(_) if is_git_dir(&git_dir) => submodule_dirs(holder, git_dir).map_err(fault),
        Err(_) => Err(fault(
            "names neither a linked worktree's git directory (no commondir) nor a submodule's",
        )),
    }
}

/// `bytes` with every `\r` and `\n` at its end taken off, as git trims a
/// `.git` file's and a worktree's `commondir` line.
const fn trim_line_ends(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., b'\n' | b'\r'] = bytes {
        bytes = rest;
    }
    bytes
}

/// Whether `dir` holds a git directory's own files.
fn is_git_dir(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

/// A linked worktree's git directory `git_dir`, whose `commondir` holds
/// `commondir`, and its common directory, when they lead back to `dot_git`.
fn worktree_dirs(
    dot_git: &Path,
    git_dir: PathBuf,
    commondir: &[u8],
) -> Result<Vec<PathBuf>, &'static str> {
    let relative = trim_line_ends(commondir);
    let common = std::fs::canonicalize(git_dir.join(OsStr::from_bytes(relative)))
        .map_err(|_| "names a worktree whose commondir doesn't resolve")?;
    if git_dir.parent() != Some(common.join("worktrees").as_path()) {
        return Err("names a git directory that isn't <common>/worktrees/<name> of its commondir");
    }
    if common.file_name() != Some(OsStr::new(".git")) && !is_git_dir(&common) {
        return Err(
            "names a worktree whose commondir is neither a .git directory nor a bare repository",
        );
    }
    // relative from the git directory, as `worktree.useRelativePaths`
    // writes it
    let back = read_small(&git_dir.join("gitdir")).ok().and_then(|back| {
        let back = trim_line_ends(&back).to_vec();
        std::fs::canonicalize(git_dir.join(OsStr::from_bytes(&back))).ok()
    });
    if back.is_none() || back != std::fs::canonicalize(dot_git).ok() {
        return Err("names a worktree whose gitdir file doesn't lead back to this .git file");
    }
    Ok(vec![git_dir, common])
}

/// A submodule's git directory `git_dir`, when it lies beneath
/// `<super>/.git/modules/` and its config's `core.worktree` leads back to
/// `checkout`, the directory holding its `.git` file.
fn submodule_dirs(checkout: &Path, git_dir: PathBuf) -> Result<Vec<PathBuf>, &'static str> {
    // `<super>/.git/modules/`, or a linked worktree's
    // `<super>/.git/worktrees/<w>/modules/`, nested modules beneath either
    let beneath_modules = git_dir
        .ancestors()
        .skip(1)
        .find(|dir| dir.file_name() == Some(OsStr::new(".git")))
        .and_then(|dot_git| git_dir.strip_prefix(dot_git).ok())
        .is_some_and(|beneath| {
            let parts: Vec<&OsStr> = beneath.iter().collect();
            matches!(parts.as_slice(), [first, _, ..] if *first == "modules")
                || matches!(
                    parts.as_slice(),
                    [first, _, third, _, ..] if *first == "worktrees" && *third == "modules"
                )
        });
    if !beneath_modules {
        return Err(
            "names a git directory that isn't a submodule's, beneath <super>/.git/modules/ or a \
             linked worktree's <super>/.git/worktrees/<w>/modules/",
        );
    }
    let config = read_small(&git_dir.join("config"))
        .map_err(|_| "names a submodule's git directory with no config it can read")?;
    let worktree = parse_config(&config)
        .map_err(|_| "names a submodule's git directory whose config git couldn't read")?
        .into_iter()
        .rev()
        .find_map(|(key, value)| (key == Key::Worktree).then_some(value))
        .ok_or("names a submodule's git directory whose config sets no core.worktree")?;
    let back = std::fs::canonicalize(git_dir.join(OsStr::from_bytes(&worktree))).ok();
    if back.as_deref() != std::fs::canonicalize(checkout).ok().as_deref() {
        return Err("names a submodule whose core.worktree doesn't lead back to this checkout");
    }
    Ok(vec![git_dir])
}
