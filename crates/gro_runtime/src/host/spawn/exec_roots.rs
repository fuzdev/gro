//! Exec roots: the directories a scope's `program_in` and `find_in` find a
//! program beneath, and the resolution of a name beneath one.
//!
//! Each exec root is opened by its real path as the run starts, beside the
//! pins, refusing any symlink on it, and held to the identity the embedder
//! read for it ([`ExecRootGrant`](crate::ExecRootGrant)): a directory
//! replaced between, gone, or no longer a directory is held as why, and
//! every call that would find a program beneath it fails with that as its
//! cause, `not_executable`: a replaced exec root refuses what it holds
//! rather than yield to an exec root holding it, whose grant may be wider
//! than its own. One whose real path isn't UTF-8 holds nothing, since no
//! directory's path a string names lies beneath it. An unconfined run's
//! scope holds one, `/`.
//!
//! `program_in(dir, name)` takes the directory of `dir`, an fs handle, by
//! its real path, read from the handle's state through the fs row
//! (`fs::handle_dir_path`), and finds the exec root of the root's scope
//! that holds that path by its text, the deepest one. No exec root holding
//! it is `not_granted`, needing an exec root. Then everything opens beneath
//! the exec root's own descriptor by `openat2` (`gro_sys::beneath`):
//!
//! - the handle's directory, by its real path beneath the exec root, with
//!   no symlink allowed on the way, so a directory moved away and a symlink
//!   left at its path is refused (`not_granted`), never followed;
//! - then `name` beneath that directory with `RESOLVE_BENEATH`, its final
//!   symlink followed, as the Node module's `program_in` resolves it by
//!   the handle's rules: an absolute name, or a `..` above the directory,
//!   refused by its text before anything is looked up, and an absolute
//!   symlink or a relative one that climbs out of the directory refused by
//!   the kernel, each `not_granted`; nothing there, or a path through a
//!   file, is no program (`find_in`'s `null`); any other failure is
//!   `not_executable`, a magic link among them, which the kernel refuses
//!   as a loop. `RESOLVE_BENEATH` rather than `RESOLVE_IN_ROOT`,
//!   since the Node module refuses a symlink that leaves the directory,
//!   where `RESOLVE_IN_ROOT` would read its target as a path beneath it,
//!   another file than the one it names.
//!
//! What's opened is a path-only descriptor of the file found, judged
//! through that descriptor (a regular file, which this user may execute),
//! and pinned from it ([`pins::pin_held`]): its identity, its change time,
//! its first bytes, and, for a native binary, the descriptor it's launched
//! by, so the file judged is the file that runs, as a program `program`
//! pins does. Its `argv[0]`, and a script's path, is the path it was found
//! at, the directory's real path joined with the name resolved by its
//! text, absolute, as the Node module's.

use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd};
use std::path::Path;

use gro_sys::beneath::{self, Links, OpenAs};

use super::pins::{self, Climb, Fault, Pin};
use crate::file_id::FileId;
use crate::{EnvPattern, GrantedDir};

/// An exec root of a scope.
#[derive(Debug)]
pub struct ExecRoot {
    /// Its absolute real path, which a directory's real path is matched
    /// against by its text; none when it isn't UTF-8, so it holds nothing.
    path: Option<String>,
    /// Its descriptor, opened as the run started, or why it isn't open.
    fd: Result<OwnedFd, String>,
    /// What a program found beneath it may be passed of the environment,
    /// `None` in an unconfined scope, which passes anything.
    pub forward: Option<Vec<EnvPattern>>,
}

impl ExecRoot {
    /// The exec root granted as `granted`, named `dir` as the run started,
    /// opened now by its real path, no symlink allowed on it, and held to
    /// `dir`'s identity; `forward` what a program found beneath it may be
    /// passed.
    pub fn open(granted: &Path, dir: &GrantedDir, forward: Vec<EnvPattern>) -> Self {
        let what = format!("the exec root granted as {}", granted.display());
        let path = dir.real.to_str().map(ToOwned::to_owned);
        let fd = path
            .as_ref()
            .ok_or_else(|| format!("{what} isn't UTF-8, so no string names it"))
            .and_then(|_| held(&what, &dir.real, Some(&dir.id)));
        Self {
            path,
            fd,
            forward: Some(forward),
        }
    }

    /// An unconfined scope's one exec root, `/`, which holds every
    /// directory and passes anything.
    pub fn whole() -> Self {
        Self {
            path: Some("/".to_owned()),
            fd: held("the filesystem's root", Path::new("/"), None),
            forward: None,
        }
    }

    /// An exec root of the real path `path` that couldn't be opened, as
    /// `why` says, for a test of what it refuses.
    #[cfg(test)]
    pub fn unopened(path: &str, why: &str, forward: &[&str]) -> Self {
        Self {
            path: Some(path.to_owned()),
            fd: Err(why.to_owned()),
            forward: Some(
                forward
                    .iter()
                    .map(|pattern| EnvPattern::parse(pattern).unwrap())
                    .collect(),
            ),
        }
    }

    /// An exec root of the real path `path`, opened now, for a test.
    #[cfg(test)]
    pub fn opened(path: &str, forward: &[&str]) -> Self {
        Self {
            path: Some(path.to_owned()),
            fd: held("a test's exec root", Path::new(path), None),
            forward: Some(
                forward
                    .iter()
                    .map(|pattern| EnvPattern::parse(pattern).unwrap())
                    .collect(),
            ),
        }
    }

    /// The path `dir`, a directory's absolute real path, has beneath it,
    /// empty for its own, or `None` when it isn't beneath it, as no path is
    /// beneath one with no path.
    fn beneath<'d>(&self, dir: &'d str) -> Option<&'d str> {
        let path = self.path.as_deref()?;
        beneath::strip_root(path.as_ref(), dir.as_ref()).and_then(|rel| rel.to_str())
    }

    /// `name` resolved beneath the directory whose real path is `dir`,
    /// which this exec root holds, and pinned: `None` when nothing
    /// runnable is there.
    ///
    /// # Errors
    ///
    /// Returns the [`Fault`] a call asking for `name` meets: `not_granted`
    /// for a name or a directory that leads out of the directory, and
    /// `not_executable` for an exec root that isn't open, a lookup that
    /// failed another way, or what can't be run.
    pub fn resolve(&self, name: &str, dir: &str) -> Result<Option<Pin>, Fault> {
        let unresolved = |why: String| Fault::Unresolved {
            cause: format!("{name} can't be resolved beneath its directory handle"),
            why,
        };
        let outside = || Fault::NotGranted(format!("{name} leads outside its directory handle"));
        if pins::is_refused_extension(name) {
            return Err(pins::refused_extension(name, name));
        }
        // the handle's refusals by text, before anything is looked up, as
        // the Node module's come
        if name.starts_with('/') {
            return Err(outside());
        }
        let Some(segments) = pins::lexical_segments(name, Climb::Refuse) else {
            return Err(outside());
        };
        let root = self.fd.as_ref().map_err(|why| unresolved(why.clone()))?;
        let rel_dir = self
            .beneath(dir)
            .ok_or_else(|| unresolved(format!("{dir} isn't beneath its exec root")))?;
        // the handle's own directory, by its real path, no symlink on it
        let opened_dir = if rel_dir.is_empty() {
            None
        } else {
            match beneath::open(
                root.as_fd(),
                Path::new(rel_dir),
                OpenAs::Directory,
                Links::Refused,
            ) {
                Ok(fd) => Some(fd),
                Err(error) if beneath::is_escape(&error) || is_symlink(&error) => {
                    return Err(outside());
                }
                Err(error) if is_missing(&error) => return Ok(None),
                Err(error) => return Err(unresolved(error.to_string())),
            }
        };
        let dir_fd: BorrowedFd<'_> = opened_dir
            .as_ref()
            .map_or_else(|| root.as_fd(), |fd| fd.as_fd());
        // a trailing `/` or `/.` passes through its last component, which
        // only a directory can be, and an emptied name is the directory
        if segments.is_empty() || name.ends_with('/') || name.ends_with("/.") {
            return Ok(None);
        }
        let rel = segments.join("/");
        let found = if dir == "/" {
            format!("/{rel}")
        } else {
            format!("{dir}/{rel}")
        };
        let held = match beneath::open(dir_fd, Path::new(&rel), OpenAs::Path, Links::Beneath) {
            Ok(fd) => fd,
            Err(error) if beneath::is_escape(&error) => return Err(outside()),
            Err(error) if is_missing(&error) => return Ok(None),
            Err(error) => return Err(unresolved(error.to_string())),
        };
        let through = format!("/proc/self/fd/{}", held.as_raw_fd());
        let metadata =
            std::fs::metadata(&through).map_err(|error| unresolved(error.to_string()))?;
        if !metadata.is_file() {
            return Ok(None);
        }
        match gro_sys::may_execute(Path::new(&through)) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                return Err(Fault::NotExecutable(format!("{found} isn't executable")));
            }
        }
        let real = std::fs::read_link(&through)
            .ok()
            .and_then(|real| real.into_os_string().into_string().ok())
            .ok_or_else(|| {
                Fault::NotExecutable(format!(
                    "{name} at {found} has no real path a string can name"
                ))
            })?;
        if pins::is_refused_extension(&real) {
            return Err(pins::refused_extension(name, &real));
        }
        pins::pin_held(name, held, real, &found, &found).map(Some)
    }
}

/// Opens `path`, an absolute real path, as a directory to resolve beneath,
/// no symlink allowed on it, and, given `expected`, holds it to that
/// identity, read from the descriptor; or why it isn't open, `what` naming
/// it.
fn held(what: &str, path: &Path, expected: Option<&FileId>) -> Result<OwnedFd, String> {
    let fd = beneath::open_root(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            format!("{what} is gone, or no longer a directory")
        }
        _ => format!("{what} can't be opened: {error}"),
    })?;
    let Some(expected) = expected else {
        return Ok(fd);
    };
    let file = std::fs::File::from(fd);
    let found = file
        .metadata()
        .map(|metadata| FileId::of(&metadata))
        .map_err(|error| format!("{what} can't be identified: {error}"))?;
    if &found != expected {
        return Err(format!("{what} was replaced since the run was granted it"));
    }
    Ok(OwnedFd::from(file))
}

/// Whether `error` is a symlink refused on a path that allows none.
fn is_symlink(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

/// Whether `error` is nothing at a path, or a path through a file.
fn is_missing(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR))
}

/// The exec root of `roots` holding `dir`, a directory's absolute real
/// path, by its text: the deepest one, or `None` when none holds it.
pub fn holding<'r>(roots: &'r [ExecRoot], dir: &str) -> Option<&'r ExecRoot> {
    roots
        .iter()
        .filter(|root| root.beneath(dir).is_some())
        .max_by_key(|root| root.path.as_ref().map_or(0, String::len))
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    use super::{ExecRoot, holding};
    use crate::GrantedDir;
    use crate::file_id::FileId;
    use crate::host::spawn::pins::Fault;
    use crate::test_support::TestDir;

    /// Writes an executable `#!` script at `path`.
    fn script(path: &Path) {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The directory at `path` as the embedder grants it now.
    fn granted(path: &Path) -> GrantedDir {
        GrantedDir {
            real: path.to_owned(),
            id: FileId::of(&std::fs::metadata(path).unwrap()),
        }
    }

    /// An exec root replaced since it was granted, by another directory at
    /// its path, holds that as why, and every name beneath it fails naming
    /// it, `not_executable`; the directory still there resolves.
    #[test]
    fn a_replaced_exec_root_is_refused() {
        let dir = TestDir::on_disk("spawn_exec_root_replaced");
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        script(&root.join("tool"));
        let held = granted(&root);
        let text = root.to_str().unwrap();
        let open = ExecRoot::open(&root, &held, Vec::new());
        assert!(matches!(open.resolve("tool", text), Ok(Some(_))));
        std::fs::rename(&root, dir.path().join("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        script(&root.join("tool"));
        let replaced = ExecRoot::open(&root, &held, Vec::new());
        match replaced.resolve("tool", text) {
            Err(Fault::Unresolved { cause, why }) => {
                assert_eq!(cause, "tool can't be resolved beneath its directory handle");
                assert!(
                    why.ends_with("was replaced since the run was granted it"),
                    "{why}"
                );
            }
            other => panic!("a replaced exec root resolved {other:?}"),
        }
    }

    /// A handle's directory moved away, a symlink left at its path, is
    /// reached through a symlink now, which leads out of the handle:
    /// `not_granted`, never followed; one gone is no program.
    #[test]
    fn a_directory_moved_and_linked_back_is_refused() {
        let dir = TestDir::on_disk("spawn_exec_root_moved");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        script(&sub.join("tool"));
        let root = ExecRoot::open(dir.path(), &granted(dir.path()), Vec::new());
        let sub_text = sub.to_str().unwrap();
        assert!(matches!(root.resolve("tool", sub_text), Ok(Some(_))));
        std::fs::rename(&sub, dir.path().join("elsewhere")).unwrap();
        assert!(matches!(root.resolve("tool", sub_text), Ok(None)));
        std::os::unix::fs::symlink("elsewhere", &sub).unwrap();
        assert_eq!(
            root.resolve("tool", sub_text).map(|pin| pin.is_some()),
            Err(Fault::NotGranted(
                "tool leads outside its directory handle".to_owned()
            ))
        );
    }

    /// An exec root nested in another and replaced since it was granted
    /// still holds what's beneath it, refusing it with the replacement as
    /// why, `not_executable`, never yielding it to the exec root holding
    /// it, which would resolve it.
    #[test]
    fn a_replaced_nested_exec_root_refuses_what_it_holds() {
        let dir = TestDir::on_disk("spawn_exec_root_replaced_nested");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let held_sub = granted(&sub);
        std::fs::rename(&sub, dir.path().join("moved")).unwrap();
        std::fs::create_dir(&sub).unwrap();
        script(&sub.join("tool"));
        let roots = [
            ExecRoot::open(dir.path(), &granted(dir.path()), Vec::new()),
            ExecRoot::open(&sub, &held_sub, Vec::new()),
        ];
        let sub_text = sub.to_str().unwrap();
        assert!(matches!(roots[0].resolve("tool", sub_text), Ok(Some(_))));
        let holder = holding(&roots, sub_text).unwrap();
        assert_eq!(holder.path.as_deref(), Some(sub_text));
        match holder.resolve("tool", sub_text) {
            Err(Fault::Unresolved { why, .. }) => assert!(
                why.ends_with("was replaced since the run was granted it"),
                "{why}"
            ),
            other => panic!("a replaced nested exec root resolved {other:?}"),
        }
    }

    /// A name's refusals by its text come before anything is looked up:
    /// an absolute name and a climb are `not_granted` beneath a directory
    /// gone since, and beneath an exec root that isn't open, as the Node
    /// module refuses them first.
    #[test]
    fn a_name_is_refused_by_its_text_first() {
        let dir = TestDir::on_disk("spawn_exec_root_text_first");
        let root = ExecRoot::open(dir.path(), &granted(dir.path()), Vec::new());
        let here = dir.path().to_str().unwrap();
        let gone = dir.path().join("gone");
        let gone = gone.to_str().unwrap();
        let unopened = ExecRoot::unopened(here, "test", &[]);
        for name in ["/bin/sh", "../x", "a/../../x"] {
            let outside = Err(Fault::NotGranted(format!(
                "{name} leads outside its directory handle"
            )));
            assert_eq!(root.resolve(name, gone).map(|pin| pin.is_some()), outside);
            assert_eq!(
                unopened.resolve(name, here).map(|pin| pin.is_some()),
                outside
            );
        }
    }

    /// A relative symlink in the handle's directory that climbs out of it
    /// leads outside the handle, `not_granted`, though its target is
    /// beneath the exec root, as the Node module's handle refuses it.
    #[test]
    fn a_link_climbing_out_of_the_handle_is_refused_within_the_exec_root() {
        let dir = TestDir::on_disk("spawn_exec_root_link_up");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        script(&dir.path().join("tool"));
        std::os::unix::fs::symlink("../tool", sub.join("up")).unwrap();
        let root = ExecRoot::open(dir.path(), &granted(dir.path()), Vec::new());
        assert!(matches!(
            root.resolve("tool", dir.path().to_str().unwrap()),
            Ok(Some(_))
        ));
        assert_eq!(
            root.resolve("up", sub.to_str().unwrap())
                .map(|pin| pin.is_some()),
            Err(Fault::NotGranted(
                "up leads outside its directory handle".to_owned()
            ))
        );
    }

    /// An exec root whose real path isn't UTF-8 holds no directory, so one
    /// beneath no other exec root is held by none, and `not_granted`,
    /// never failed through it.
    #[test]
    fn an_exec_root_no_string_names_holds_nothing() {
        let real = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/nu\xff"));
        let unnamed = ExecRoot::open(
            &real,
            &GrantedDir {
                real: real.clone(),
                id: FileId::of(&std::fs::metadata("/").unwrap()),
            },
            Vec::new(),
        );
        assert!(unnamed.path.is_none());
        let roots = [unnamed, ExecRoot::unopened("/a", "test", &[])];
        assert!(holding(&roots, "/x").is_none());
        assert!(holding(&roots, "/").is_none());
        assert_eq!(
            holding(&roots, "/a/b").and_then(|root| root.path.as_deref()),
            Some("/a")
        );
    }

    /// The exec root holding a directory is the deepest one whose path it
    /// lies beneath, by whole names: none holds a sibling sharing a prefix
    /// of its text.
    #[test]
    fn the_deepest_exec_root_holds_a_directory() {
        let roots = [
            ExecRoot::unopened("/a", "test", &[]),
            ExecRoot::unopened("/a/b", "test", &[]),
            ExecRoot::unopened("/", "test", &[]),
        ];
        let held = |dir: &str| holding(&roots, dir).and_then(|root| root.path.as_deref());
        assert_eq!(held("/a/b/c"), Some("/a/b"));
        assert_eq!(held("/a/b"), Some("/a/b"));
        assert_eq!(held("/a/bc"), Some("/a"));
        assert_eq!(held("/x"), Some("/"));
        assert!(holding(&roots[..2], "/ab").is_none());
        assert!(holding(&roots[..2], "/x").is_none());
    }
}
