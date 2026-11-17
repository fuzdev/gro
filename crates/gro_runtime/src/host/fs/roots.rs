//! The roots are the directories the run reads or writes, each opened once,
//! as a path only (`O_PATH | O_DIRECTORY`), by its real path with no
//! symlink on it, the first time a run makes the fs namespace
//! ([`FsState`]): the project's root, the entry's, first, read and never
//! written but by a grant; then each directory the run's fs grant reads or
//! writes ([`FsGrant`]), one root per real path, its scopes joined; or,
//! unconfined, one more at `/`, read and written. A root matches a path by
//! its real path and by each path it was granted as, so a directory
//! granted by a path through a symlink is held by that path too, while
//! every lookup resolves beneath its descriptor.
//!
//! The roots are one run table ([`Table`]), only ever appended to, so a
//! handle's place in it stays its root's: each root's descriptor, its real
//! path and the paths it was granted as, and the roots holding its
//! directory as the table was made. What a root reads and writes is a
//! view's ([`View`]): a list of places in the table, each with the scope
//! its grants give it there, joined with the scope of each root of the
//! same view that holds its directory, so a directory written inside a
//! read one is read too, and write never implies read. Scopes join within
//! a view only. A run has one view today, the project's, holding every
//! root, so its join is the run's; a view per origin would join only its
//! own grants, since a join across two would hand one origin's scope to
//! another's root. An op that reaches outside a handle (`fs.open_dir`,
//! `fs.real_path`) and every refusal's hint read the view; whether a path
//! is or holds a root reads the table, whoever's root it is.
//!
//! No `remove` or `rename` removes or moves a root's directory, or a
//! directory holding one, and no `rename` moves an entry where the deepest
//! root holding it grants what the source's doesn't, in any view
//! ([`Held`]): a rule the roots keep whatever the run grants, `-A`
//! included, not a grant, and the one place an op reads the run's roots to
//! decide, only ever to refuse; beside it, a refusal's hint reads them, to
//! name what would let the op through ([`super::walk::scope_needed`]).
//!
//! A granted directory is the one its path named as the run started
//! ([`FsGrant`]): the embedder reads its real path then, and the identity
//! of the directory there, and its root opens by that real path, never by
//! where the granted path leads at the first import. Its descriptor's
//! identity, read once it opens (one `statx`, only for a root a granted
//! path is held to), must be that one, or the path is a root of its own
//! held as why, replaced since, as one gone from its real path is, whose
//! grants join no other root's. So a directory swapped in at a granted
//! directory's real path between the start and the first import is never
//! a root, a granted symlink pointed elsewhere since still names the
//! directory it named, and a directory moved away since is gone. The
//! identity is the device, the inode, and the birth time where the
//! filesystem records one: a directory removed and another made at its
//! path can take the inode number, and passes where the filesystem records
//! no birth time or stamps both within one tick of its clock. The check
//! holds the root as it opens and nothing after: the root's descriptor is
//! the directory from then on. The project's own read is held to none (a
//! flag naming its directory is): the loader finds it by its path and
//! loads every module from it by its path, so a directory swapped in there
//! is the run's project for the loader too.
//!
//! A root holds another's directory when the other's real path, the
//! start's, lies beneath its own and, both open, the directory at that path
//! beneath its descriptor, no symlink on the way, is the other's by its
//! identity, as the table is made. So the nesting the scopes join by is
//! the one the run started with, each pair of it still in place at the
//! first import: a flag's directory moved beneath a root in between, its
//! symlink pointed along, is gone from its real path and joins nothing, a
//! directory moved between the opens takes nothing of a root's scope, and
//! a root that isn't open lends its scope to none. Nothing is read again
//! once the table is made: a root's directory moved out from beneath
//! another after that keeps the scope it joined, for every handle on it,
//! and one moved beneath another takes none of its scope.

use std::cell::Ref;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{FsState, Scope, sys};
use crate::file_id::FileId;
use crate::host::HostData;
use crate::{Authority, FsGrant};

/// Why the root table's one root isn't open in a runtime given no
/// project, which only a runtime made outside a run's start is.
const NO_PROJECT: &str = "this runtime was started without a project";

/// A root of the run: a directory whose tree it reads or writes.
#[derive(Debug)]
pub(super) struct Root {
    /// Its absolute real path, which the paths of its handles extend; lossy
    /// only when its path isn't UTF-8, when it isn't open; none for a
    /// granted path refused as replaced, which only the path it was granted
    /// as matches, and for a runtime given no project, which no path
    /// matches.
    path: Option<String>,
    /// The other absolute paths it was granted as, each by its text with no
    /// `.`, `..`, or empty name, which match a path as its real path does.
    granted: Vec<String>,
    /// The places of the roots holding its directory as the table was
    /// made ([`holds_beneath`]), whose scopes a view joins to its own.
    holders: Vec<usize>,
    /// Its descriptor, a path only, shared with any op under way, or why it
    /// isn't open, which every op through it reports.
    pub(super) fd: Result<Arc<OwnedFd>, String>,
}

impl Root {
    /// A root at `path`, never opened, granted as `granted` too, with the
    /// scope a test's view gives it.
    #[cfg(test)]
    pub(super) fn unopened(path: &str, granted: &[&str], scope: Scope) -> (Self, Scope) {
        let (root, scope) = Self::unplaced(granted, scope);
        (
            Self {
                path: Some(path.to_owned()),
                ..root
            },
            scope,
        )
    }

    /// A root with no real path, never opened, granted as `granted`, with
    /// the scope a test's view gives it.
    #[cfg(test)]
    pub(super) fn unplaced(granted: &[&str], scope: Scope) -> (Self, Scope) {
        (
            Self {
                path: None,
                granted: granted.iter().map(|&path| path.to_owned()).collect(),
                holders: Vec::new(),
                fd: Err("not opened".to_owned()),
            },
            scope,
        )
    }

    /// Its absolute real path, none for a root no path matches as its own.
    pub(super) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The path a path beneath it extends: its real path, or empty with
    /// none, which only the project's handle in a runtime given no project
    /// extends, since a root with no real path never opens, and a handle
    /// on any other root is made through its open descriptor.
    pub(super) fn handle_base(&self) -> &str {
        self.path().unwrap_or_default()
    }

    /// Whether it's `/`, which resolves an absolute symlink and a `..` as
    /// the kernel resolves a path (`gro_sys`'s `Links::InRoot`).
    pub(super) fn whole(&self) -> bool {
        self.path() == Some("/")
    }

    /// Each absolute path it matches as, its real path first, as its names.
    pub(super) fn spellings(&self) -> impl Iterator<Item = Vec<&str>> {
        self.path()
            .into_iter()
            .chain(self.granted.iter().map(String::as_str))
            .map(|path| path.split('/').filter(|name| !name.is_empty()).collect())
    }
}

/// The run's roots: the run table, which handles index, and the project's
/// view of it, the one view a run has.
#[derive(Debug)]
pub(super) struct Table {
    /// Every root of the run, by its place, only ever appended to.
    roots: Vec<Root>,
    /// What the project reads and writes of the roots.
    project: View,
}

/// What one origin reads and writes of the run's roots: places in the
/// table, each with its scope, its own grants' joined with each root's of
/// this view that holds its directory, and no other view's.
#[derive(Debug, Default)]
pub(super) struct View {
    /// Each root of the view, by its place in the table, with its scope.
    entries: Vec<(usize, Scope)>,
}

impl View {
    /// The view of `granted`, places in `roots` with the scope each one's
    /// grants give it, each scope joined with every one of `granted`'s
    /// whose root holds its root's directory ([`Root::holders`]).
    fn joined(roots: &[Root], granted: &[(usize, Scope)]) -> Self {
        let entries = granted
            .iter()
            .map(|&(place, scope)| {
                let holders = roots.get(place).map_or(&[][..], |root| &root.holders);
                let joined = granted
                    .iter()
                    .filter(|(outer, _)| holders.contains(outer))
                    .fold(scope, |scope, &(_, outer)| scope.join(outer));
                (place, joined)
            })
            .collect();
        Self { entries }
    }

    /// Each root of the view, its place and scope, in the table's order.
    pub(super) fn entries(&self) -> impl Iterator<Item = (usize, Scope)> + '_ {
        self.entries.iter().copied()
    }

    /// The scope the view gives the root at `place`, none when it holds no
    /// such root.
    pub(super) fn scope_of(&self, place: usize) -> Scope {
        self.entries
            .iter()
            .find(|&&(held, _)| held == place)
            .map_or_else(Scope::default, |&(_, scope)| scope)
    }
}

impl Table {
    /// The roots, by their places.
    pub(super) fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// The project's view, the one a run has.
    pub(super) const fn project(&self) -> &View {
        &self.project
    }

    /// Every view of the run, which the rule's scope gain is checked in
    /// ([`Held`]): the project's alone.
    pub(super) fn views(&self) -> impl Iterator<Item = &View> {
        std::iter::once(&self.project)
    }

    /// A table of `roots`, each given its scope as a test's view gives it,
    /// joined already.
    #[cfg(test)]
    pub(super) fn of(roots: impl IntoIterator<Item = (Root, Scope)>) -> Self {
        let (roots, scopes): (Vec<Root>, Vec<Scope>) = roots.into_iter().unzip();
        Self {
            roots,
            project: View {
                entries: scopes.into_iter().enumerate().collect(),
            },
        }
    }
}

impl FsState {
    /// The run's roots, opened now if they weren't, from the run's data
    /// and authority ([`root_table`]).
    pub(super) fn roots(&self, data: &HostData, authority: Option<&Authority>) -> Ref<'_, Table> {
        if self.roots.borrow().is_none() {
            let table = root_table(data.project.as_deref(), authority);
            *self.roots.borrow_mut() = Some(table);
        }
        Ref::map(self.roots.borrow(), |roots| {
            roots.as_ref().unwrap_or(&EMPTY_TABLE)
        })
    }
}

/// The table a runtime given no state reads, never one a run makes.
static EMPTY_TABLE: Table = Table {
    roots: Vec::new(),
    project: View {
        entries: Vec::new(),
    },
};

/// Whether `path`, an absolute path, is a root's directory or holds one,
/// by its text against each root's real path as the run started, whoever's
/// root it is.
pub(super) fn holds_a_root(roots: &[Root], path: &str) -> bool {
    roots
        .iter()
        .any(|root| root.path().is_some_and(|dir| at_or_beneath(dir, path)))
}

/// Whether `path` is `dir` or lies beneath it, both absolute, by their
/// text.
fn at_or_beneath(path: &str, dir: &str) -> bool {
    dir == "/"
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Why the rule refuses a remove, or either end of a rename: the entry is a
/// root's directory or holds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HoldsARoot;

/// An end of a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenameEnd {
    /// The entry moved, the source.
    From,
    /// Where it moves, the destination.
    To,
}

/// Why the rule refuses a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MoveRefusal {
    /// This end is a root's directory or holds one.
    HoldsARoot(RenameEnd),
    /// The destination's deepest root grants what the source's doesn't.
    GainsScope,
}

/// The roots a rename or a remove beneath one handle's directory can
/// reach, owned, so the op's I/O can check its ends in the directories it
/// acts in on whatever thread it runs: the real path of each root of the
/// table beneath the directory's, and, for each view, the scope it gives
/// each of those it holds and the deepest root holding the directory.
///
/// An entry beneath a handle's directory is a root's directory or holds
/// one only when a root lies beneath that directory, and its deepest root
/// in a view is either such a root or the directory's own, so a handle's
/// directory with no root beneath it needs no check at all ([`Held::of`]).
/// An end's path is its real path, read through the directory the op acts
/// in, and the roots' are their real paths as the run started: the rule
/// holds each root's directory where it was, which nothing in the run can
/// move (`authority.md`'s threat model leaves out a process outside the
/// run that moves what a flag names). Only a directory can be or hold one,
/// and an entry at a root's path as the run started is that directory, so
/// the text alone decides, and nothing is looked up for it. It compares
/// paths, not directories: a root's directory a bind mount shows at a
/// second path is no root at that path, and through it the rule neither
/// keeps the directory nor holds what moves into it.
#[derive(Debug)]
pub(super) struct Held {
    /// The real path of each root beneath the handle's directory.
    beneath: Vec<String>,
    /// For each view, what it gives there ([`Gains`]).
    views: Vec<Gains>,
}

/// What one view gives beneath a handle's directory: the scope of its
/// deepest root holding the directory, and each of its roots beneath the
/// directory, by real path, with its scope.
#[derive(Debug)]
struct Gains {
    scope: Scope,
    beneath: Vec<(String, Scope)>,
}

impl Gains {
    /// The scope the view gives `path`, a real path beneath the handle's
    /// directory: its deepest root holding it.
    fn scope_at(&self, path: &str) -> Scope {
        self.beneath
            .iter()
            .filter(|(root, _)| at_or_beneath(path, root))
            .max_by_key(|(root, _)| root.len())
            .map_or(self.scope, |(_, scope)| *scope)
    }
}

impl Held {
    /// The roots beneath `dir`, a handle's directory's absolute path, of
    /// `table` and of each of its views, or none when no root lies beneath
    /// it.
    pub(super) fn of(table: &Table, dir: &str) -> Option<Self> {
        let roots = table.roots();
        let below = |path: &str| path != dir && at_or_beneath(path, dir);
        let beneath: Vec<String> = roots
            .iter()
            .filter_map(Root::path)
            .filter(|path| below(path))
            .map(ToOwned::to_owned)
            .collect();
        if beneath.is_empty() {
            return None;
        }
        let views = table
            .views()
            .map(|view| {
                let placed = || {
                    view.entries()
                        .filter_map(|(place, scope)| Some((roots.get(place)?.path()?, scope)))
                };
                let scope = placed()
                    .filter(|&(path, _)| at_or_beneath(dir, path))
                    .max_by_key(|&(path, _)| path.len())
                    .map_or_else(Scope::default, |(_, scope)| scope);
                let beneath = placed()
                    .filter(|&(path, _)| below(path))
                    .map(|(path, scope)| (path.to_owned(), scope))
                    .collect();
                Gains { scope, beneath }
            })
            .collect();
        Some(Self { beneath, views })
    }

    /// That the entry at `path`, a real path beneath the handle's
    /// directory, may be removed, or moved: it's no root's directory and
    /// holds none.
    pub(super) fn removable(&self, path: &str) -> Result<(), HoldsARoot> {
        if self.beneath.iter().any(|root| at_or_beneath(root, path)) {
            return Err(HoldsARoot);
        }
        Ok(())
    }

    /// That the entry at `from` may be moved to `to`, both real paths
    /// beneath the handle's directory: neither is a root's directory or
    /// holds one, the destination checked though nothing is there, as a
    /// rename over an empty directory replaces it; and in no view does the
    /// deepest root holding the destination grant what the source's
    /// doesn't, as Landlock's `REFER` holds a rename, so no write moves a
    /// file into a root that reads it. A refusal of a root's directory
    /// names its end.
    pub(super) fn movable(&self, from: &str, to: &str) -> Result<(), MoveRefusal> {
        self.removable(from)
            .map_err(|HoldsARoot| MoveRefusal::HoldsARoot(RenameEnd::From))?;
        self.removable(to)
            .map_err(|HoldsARoot| MoveRefusal::HoldsARoot(RenameEnd::To))?;
        for view in &self.views {
            let (source, destination) = (view.scope_at(from), view.scope_at(to));
            if (destination.read && !source.read) || (destination.write && !source.write) {
                return Err(MoveRefusal::GainsScope);
            }
        }
        Ok(())
    }
}

/// A root the table wants, in the order it wants them: its real path, read
/// as the run started, none for a runtime given no project; what it reads
/// and writes; and the path it was granted as, with the identity its
/// directory had when the embedder checked it, none for the project's root
/// and `/` unconfined.
struct Wanted<'g> {
    real: Option<PathBuf>,
    scope: Scope,
    granted: Option<(&'g Path, &'g FileId)>,
}

/// A root of the table being made: its real path, none for a granted path
/// refused as replaced or a runtime given no project; its scope, before it
/// joins the scopes of the roots holding it; each path it was granted as;
/// and its descriptor with the identity of its directory, read when a
/// granted path is held to one, or why it isn't open.
struct Joined<'g> {
    real: Option<PathBuf>,
    scope: Scope,
    granted: Vec<&'g Path>,
    fd: Result<(OwnedFd, Option<FileId>), String>,
}

impl<'g> Joined<'g> {
    /// The root `want` names, opened ([`opened`]), the identity of its
    /// directory read when `identify` says a granted path is held to it.
    fn open(project: bool, want: &Wanted<'g>, identify: bool) -> Self {
        let granted: Vec<&Path> = want.granted.iter().map(|(path, _)| *path).collect();
        Self {
            fd: opened(project, want.real.as_deref(), &granted, identify),
            real: want.real.clone(),
            scope: want.scope,
            granted,
        }
    }

    /// A root of its own for `path`, held as `why`: what a granted path
    /// whose directory isn't the one it was checked as is, as one gone is.
    fn refused(path: &'g Path, scope: Scope, why: String) -> Self {
        Self {
            real: None,
            scope,
            granted: vec![path],
            fd: Err(why),
        }
    }

    /// That the directory this root opened is the one `path` named when it
    /// was granted, `expected`, else why not. A root that didn't open holds
    /// every path granted as it to its own failure.
    fn holds(&self, path: &Path, expected: &FileId) -> Result<(), String> {
        match &self.fd {
            Ok((_, found)) if found.as_ref() != Some(expected) => Err(format!(
                "the directory granted as {} was replaced since the run started",
                path.display()
            )),
            _ => Ok(()),
        }
    }
}

/// The run's roots: the project's, `project`, a real path, read; then the
/// directories `authority`'s fs grant reads and writes ([`FsGrant`]), or,
/// unconfined, `/`, read and written. Each granted path's root is the real
/// path the embedder read as the run started, never read again here, and
/// a granted path is held to the identity its directory had then: one
/// whose directory has another, replaced since, is a root of its own, held
/// as why, as one gone from its real path by now is. Each real path is
/// opened once ([`opened`]), and again by the next path granted as it when
/// the path that opened it was refused. The paths of one real path are one
/// root, the first's place, its scopes joined and each path it was granted
/// as kept; then each root records the roots that hold its directory
/// ([`holds_beneath`]), by these real paths, the start's, and the
/// descriptors now, and the project's view, holding every root, joins
/// each one's scope with theirs ([`View::joined`]).
fn root_table(project: Option<&Path>, authority: Option<&Authority>) -> Table {
    let mut wanted = vec![Wanted {
        real: project.map(Path::to_path_buf),
        scope: Scope::READ,
        granted: None,
    }];
    match authority {
        Some(Authority::Unconfined) => wanted.push(Wanted {
            real: Some(PathBuf::from("/")),
            scope: Scope::ALL,
            granted: None,
        }),
        Some(Authority::Confined(grants)) => {
            if let Some(FsGrant { read, write }) = &grants.fs {
                for (dirs, scope) in [(read, Scope::READ), (write, Scope::WRITE)] {
                    for (dir, at_start) in dirs {
                        wanted.push(Wanted {
                            real: Some(at_start.real.clone()),
                            scope,
                            granted: Some((dir.as_path(), &at_start.id)),
                        });
                    }
                }
            }
        }
        None => {}
    }
    let mut table: Vec<Joined<'_>> = Vec::new();
    for (index, want) in wanted.iter().enumerate() {
        let held = want.real.as_ref().and_then(|real| {
            table
                .iter_mut()
                .find(|joined| joined.real.as_ref() == Some(real))
        });
        if let Some(joined) = held {
            match want
                .granted
                .map(|(path, id)| (path, joined.holds(path, id)))
            {
                Some((path, Err(why))) => table.push(Joined::refused(path, want.scope, why)),
                granted => {
                    joined.scope = joined.scope.join(want.scope);
                    joined.granted.extend(granted.map(|(path, _)| path));
                }
            }
            continue;
        }
        // a directory's identity is read only when a granted path is held
        // to it
        let identify = want.real.is_some()
            && wanted
                .iter()
                .any(|other| other.real == want.real && other.granted.is_some());
        let joined = Joined::open(index == 0, want, identify);
        table.push(
            match want
                .granted
                .map(|(path, id)| (path, joined.holds(path, id)))
            {
                Some((path, Err(why))) => Joined::refused(path, want.scope, why),
                _ => joined,
            },
        );
    }
    let holders: Vec<Vec<usize>> = table
        .iter()
        .enumerate()
        .map(|(place, joined)| {
            (0..table.len())
                .filter(|&outer| outer != place && holds_beneath(&table[outer], joined))
                .collect()
        })
        .collect();
    let granted: Vec<(usize, Scope)> = table
        .iter()
        .map(|joined| joined.scope)
        .enumerate()
        .collect();
    let roots: Vec<Root> = table.into_iter().zip(holders).map(root).collect();
    let project = View::joined(&roots, &granted);
    Table { roots, project }
}

/// Whether `outer` holds `inner`'s directory: `inner` itself, or a root
/// whose real path lies beneath `outer`'s by its text and, both open,
/// whose directory is, by its identity, the one at that path beneath
/// `outer`'s descriptor, no symlink on the way ([`sys::dir_metadata`]).
/// The real paths are the start's, so the text says where the directories
/// were as the run started, and a directory moved since, or between the
/// opens, can lie beneath a root's text and not its directory: the
/// descriptors say where it is as the table is made, and both must nest. A
/// root that isn't open holds no other; one that isn't open, which reaches
/// nothing, is held by the text alone.
fn holds_beneath(outer: &Joined<'_>, inner: &Joined<'_>) -> bool {
    let (Some(outer_real), Some(real)) = (&outer.real, &inner.real) else {
        return false;
    };
    let Ok(rel) = real.strip_prefix(outer_real) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return true;
    }
    match (&outer.fd, &inner.fd) {
        (Ok((outer_fd, _)), Ok((inner_fd, id))) => {
            let found = sys::dir_metadata(outer_fd, rel).map(|metadata| FileId::of(&metadata));
            // the identity read as it opened, else read now from its
            // descriptor, which holds the directory either way
            let inner = id.map_or_else(|| identity(inner_fd), Ok);
            matches!((found, inner), (Ok(found), Ok(inner)) if found == inner)
        }
        (Err(_), _) => false,
        (Ok(_), Err(_)) => true,
    }
}

/// The identity of the directory `fd` holds.
fn identity(fd: &OwnedFd) -> std::io::Result<FileId> {
    std::fs::File::from(fd.try_clone()?)
        .metadata()
        .map(|metadata| FileId::of(&metadata))
}

/// The root `joined` makes, held by the roots at `holders`.
fn root((joined, holders): (Joined<'_>, Vec<usize>)) -> Root {
    let path = joined
        .real
        .as_deref()
        .map(|real| real.to_string_lossy().into_owned());
    let names: Vec<String> = joined
        .granted
        .iter()
        .filter_map(|path| spelled(path))
        .filter(|spelling| joined.real.as_deref().and_then(Path::to_str) != Some(spelling))
        .fold(Vec::new(), |mut names, spelling| {
            if !names.contains(&spelling) {
                names.push(spelling);
            }
            names
        });
    Root {
        path,
        granted: names,
        holders,
        fd: joined.fd.map(|(fd, _)| Arc::new(fd)),
    }
}

/// The descriptor of the directory at `real`, a real path, granted as
/// `granted`, with its identity when `identify` says so (read from the
/// descriptor, so it's the directory opened), or why it isn't open: none
/// given (no project), a path that isn't UTF-8, nothing there or no
/// directory (gone), or a failed open or identity. `project` says it's the
/// project's, which its reasons name.
fn opened(
    project: bool,
    real: Option<&Path>,
    granted: &[&Path],
    identify: bool,
) -> Result<(OwnedFd, Option<FileId>), String> {
    let what = if project {
        "the project's directory".to_owned()
    } else {
        format!("the directory granted as {}", granted_text(granted))
    };
    let Some(dir) = real else {
        return Err(NO_PROJECT.to_owned());
    };
    let path = dir.to_string_lossy();
    // the real path, unless it's the path the root was granted as
    let what = if !project && granted_text(granted) == path {
        what
    } else {
        format!("{what}, {path},")
    };
    if dir.to_str().is_none() {
        return Err(format!("{what} isn't UTF-8, so no string names it"));
    }
    let fd = sys::open_root(dir).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            format!("{what} is gone, or no longer a directory")
        }
        _ => format!("{what} can't be opened: {error}"),
    })?;
    if !identify {
        return Ok((fd, None));
    }
    // read from the descriptor, through a `File` that hands it back
    let file = std::fs::File::from(fd);
    let id = file
        .metadata()
        .map(|metadata| FileId::of(&metadata))
        .map_err(|error| format!("{what} can't be identified: {error}"))?;
    Ok((OwnedFd::from(file), Some(id)))
}

/// The paths a root was granted as, for a reason to name it by.
fn granted_text(granted: &[&Path]) -> String {
    granted
        .first()
        .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
}

/// `path`, an absolute path as granted, as a root's spelling: its names
/// joined, none `.` or empty, or none when it holds a `..`, which the
/// kernel resolves after the symlink before it, so its text names no
/// directory, or isn't UTF-8.
fn spelled(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let mut names = Vec::new();
    for name in text.split('/') {
        match name {
            "" | "." => {}
            ".." => return None,
            name => names.push(name),
        }
    }
    Some(format!("/{}", names.join("/")))
}

#[cfg(test)]
mod tests {
    use super::{
        Held, HoldsARoot, MoveRefusal, RenameEnd, Root, Scope, Table, holds_a_root, root_table,
        spelled,
    };

    /// A root of a table as a test shows it: its real path, the other
    /// paths it was granted as, its scope, and why it isn't open.
    type Shown = (Option<String>, Vec<String>, Scope, Option<String>);

    /// Each root of `table` as a test shows it, with the scope the
    /// project's view gives it.
    fn show(table: &Table) -> Vec<Shown> {
        table
            .roots()
            .iter()
            .enumerate()
            .map(|(place, root)| {
                (
                    root.path.clone(),
                    root.granted.clone(),
                    table.project().scope_of(place),
                    root.fd.as_ref().err().cloned(),
                )
            })
            .collect()
    }

    /// The roots of a run granted `--allow-write=/o --allow-read=/o/r
    /// --allow-write=/o/w/inner` with its project at `/p`, each by its real
    /// path and joined scope, one more granted as `/o/alias` and one
    /// refused as replaced, which no real path names.
    fn rule_roots() -> Table {
        Table::of([
            Root::unopened("/p", &[], Scope::READ),
            Root::unopened("/o/r", &["/o/alias"], Scope::ALL),
            Root::unopened("/o/w/inner", &[], Scope::WRITE),
            Root::unopened("/o", &[], Scope::WRITE),
            Root::unplaced(&["/o/gone"], Scope::READ),
        ])
    }

    /// A path is or holds a root by its text against the roots' real paths
    /// alone: a root's own path, any path above one, `/`, but not one
    /// beside a root, one a name longer than a root's, a path inside one,
    /// a path a root was only granted as, or one a refused root was
    /// granted as.
    #[test]
    fn a_path_is_or_holds_a_root_by_its_text() {
        let roots = rule_roots();
        for path in ["/o/r", "/o", "/o/w", "/o/w/inner", "/p", "/"] {
            assert!(holds_a_root(roots.roots(), path), "{path}");
        }
        for path in [
            "/o/x",
            "/o/rr",
            "/o/r/f",
            "/o/w/inner2",
            "/o/alias",
            "/o/gone",
            "/q",
        ] {
            assert!(!holds_a_root(roots.roots(), path), "{path}");
        }
    }

    /// The rule beneath a handle's directory: none when no root lies
    /// beneath it; else an entry that is or holds a root beneath it can't
    /// be removed or moved, at either end of a rename, and a rename can't
    /// move an entry where its deepest root grants what the source's
    /// doesn't, a read here, but can where it grants less or the same.
    #[test]
    fn a_rename_or_a_remove_beneath_a_handle_is_held_to_the_roots() {
        let roots = rule_roots();
        assert!(Held::of(&roots, "/o/r").is_none());
        assert!(Held::of(&roots, "/o/w/inner").is_none());
        assert!(Held::of(&roots, "/o/x").is_none());
        assert!(Held::of(&roots, "/q").is_none());
        let held = Held::of(&roots, "/o").unwrap();
        for path in ["/o/r", "/o/w", "/o/w/inner"] {
            assert_eq!(held.removable(path), Err(HoldsARoot), "{path}");
        }
        for path in [
            "/o/x",
            "/o/r/f",
            "/o/w/inner/f",
            "/o/w/f",
            "/o/rr",
            "/o/alias",
        ] {
            assert_eq!(held.removable(path), Ok(()), "{path}");
        }
        assert_eq!(
            held.movable("/o/w", "/o/x"),
            Err(MoveRefusal::HoldsARoot(RenameEnd::From))
        );
        assert_eq!(
            held.movable("/o/x", "/o/r"),
            Err(MoveRefusal::HoldsARoot(RenameEnd::To))
        );
        assert_eq!(
            held.movable("/o/secret", "/o/r/secret"),
            Err(MoveRefusal::GainsScope)
        );
        assert_eq!(held.movable("/o/r/f", "/o/f"), Ok(()));
        assert_eq!(held.movable("/o/f", "/o/w/inner/f"), Ok(()));
        assert_eq!(held.movable("/o/f", "/o/w/g"), Ok(()));
        let in_w = Held::of(&roots, "/o/w").unwrap();
        assert_eq!(in_w.movable("/o/w/f", "/o/w/inner/f"), Ok(()));
        assert_eq!(in_w.removable("/o/w/inner"), Err(HoldsARoot));
        // a write root inside a root that reads only: moving in gains the write
        let read_only = Table::of([
            Root::unopened("/p", &[], Scope::READ),
            Root::unopened("/p/out", &[], Scope::ALL),
        ]);
        let held = Held::of(&read_only, "/p").unwrap();
        assert_eq!(
            held.movable("/p/f", "/p/out/f"),
            Err(MoveRefusal::GainsScope)
        );
        assert_eq!(held.movable("/p/out/f", "/p/f"), Ok(()));
    }

    /// The deepest root decides a scope, an end's and the handle's
    /// directory's alike: a root inside a root beneath the handle, and a
    /// handle beneath two nested roots, each granting more than the root
    /// holding it.
    #[test]
    fn the_deepest_root_holding_a_path_decides_its_scope() {
        let roots = Table::of([
            Root::unopened("/o", &[], Scope::WRITE),
            Root::unopened("/o/w", &[], Scope::WRITE),
            Root::unopened("/o/w/r", &[], Scope::ALL),
            Root::unopened("/o/a", &[], Scope::ALL),
            Root::unopened("/o/a/x/r", &[], Scope::ALL),
        ]);
        let held = Held::of(&roots, "/o").unwrap();
        assert_eq!(
            held.movable("/o/f", "/o/w/r/f"),
            Err(MoveRefusal::GainsScope)
        );
        let in_x = Held::of(&roots, "/o/a/x").unwrap();
        assert_eq!(in_x.movable("/o/a/x/f", "/o/a/x/r/f"), Ok(()));
    }

    /// A view joins each of its roots' scope with the scope of every root
    /// of the view holding its directory, at any depth, while a view's own
    /// entries are all it joins: a root it doesn't hold lends it nothing,
    /// whoever else holds it. The table builder's view of nested granted
    /// directories is that join.
    #[test]
    fn a_view_joins_the_scopes_of_its_own_roots_holding_each() {
        use super::View;
        use crate::{Authority, FsGrant, Grants};

        // `/o` holds `/o/w`, which holds `/o/w/r`; `/q` holds none
        let roots: Vec<Root> = [vec![], vec![0], vec![0, 1], vec![]]
            .into_iter()
            .map(|holders| Root {
                holders,
                ..Root::unplaced(&[], Scope::ALL).0
            })
            .collect();
        let granted = [
            (0, Scope::READ),
            (1, Scope::WRITE),
            (2, Scope::WRITE),
            (3, Scope::WRITE),
        ];
        let view = View::joined(&roots, &granted);
        assert_eq!(
            view.entries().collect::<Vec<_>>(),
            [
                (0, Scope::READ),
                (1, Scope::ALL),
                (2, Scope::ALL),
                (3, Scope::WRITE)
            ]
        );
        // a view without `/o`: nothing joins its read
        let view = View::joined(&roots, &granted[1..3]);
        assert_eq!(
            view.entries().collect::<Vec<_>>(),
            [(1, Scope::WRITE), (2, Scope::WRITE)]
        );
        assert_eq!(view.scope_of(0), Scope::default());

        let dir = crate::test_support::TestDir::on_disk("view_join");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "o/w/r", "q"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        let authority = Authority::Confined(Grants {
            fs: Some(FsGrant {
                read: [checked(&at("o"))].into(),
                write: [
                    checked(&at("o/w")),
                    checked(&at("o/w/r")),
                    checked(&at("q")),
                ]
                .into(),
            }),
            ..Grants::default()
        });
        let table = root_table(Some(&at("p")), Some(&authority));
        let path = |name: &str| at(name).display().to_string();
        let scopes: Vec<(Option<String>, Scope)> = show(&table)
            .into_iter()
            .map(|(path, _, scope, _)| (path, scope))
            .collect();
        assert_eq!(
            scopes,
            [
                (Some(path("p")), Scope::READ),
                (Some(path("o")), Scope::READ),
                (Some(path("o/w")), Scope::ALL),
                (Some(path("o/w/r")), Scope::ALL),
                (Some(path("q")), Scope::WRITE),
            ]
        );
        let holders: Vec<&[usize]> = table
            .roots()
            .iter()
            .map(|root| root.holders.as_slice())
            .collect();
        assert_eq!(holders, [&[][..], &[], &[1], &[1, 2], &[]]);
    }

    /// Unconfined, `/` holds everything, read and written, and the
    /// project's root, inside it, can't be removed or moved through a
    /// handle above it; nothing gains a scope.
    #[test]
    fn the_rule_holds_unconfined() {
        let roots = Table::of([
            Root::unopened("/p/proj", &[], Scope::ALL),
            Root::unopened("/", &[], Scope::ALL),
        ]);
        let held = Held::of(&roots, "/p").unwrap();
        assert_eq!(held.removable("/p/proj"), Err(HoldsARoot));
        assert_eq!(held.movable("/p/f", "/p/proj/f"), Ok(()));
        assert!(Held::of(&roots, "/").is_some());
        assert!(Held::of(&roots, "/q").is_none());
    }

    /// A granted path spells its root by its names, `.` and empty ones
    /// dropped; one holding a `..`, which the kernel resolves after the
    /// symlink before it, spells none.
    #[test]
    fn a_granted_path_spells_its_root_by_its_text() {
        use std::path::Path;
        for (granted, spelling) in [
            ("/a/b", Some("/a/b")),
            ("/a//./b/", Some("/a/b")),
            ("/", Some("/")),
            ("/a/../b", None),
            ("/a/..", None),
        ] {
            assert_eq!(
                spelled(Path::new(granted)).as_deref(),
                spelling,
                "{granted}"
            );
        }
    }

    /// `path` as an embedder grants it: with its real path now and the
    /// identity of the directory there, or, when nothing is there, the path
    /// itself and an identity no directory has.
    fn checked(path: &std::path::Path) -> (std::path::PathBuf, crate::GrantedDir) {
        let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let id = std::fs::metadata(&real).map_or(
            crate::file_id::FileId {
                dev: u64::MAX,
                ino: u64::MAX,
                born: None,
            },
            |metadata| crate::file_id::FileId::of(&metadata),
        );
        (path.to_path_buf(), crate::GrantedDir { real, id })
    }

    /// A granted path whose directory at its real path isn't the one it
    /// named when it was granted, replaced since, is a root of its own held
    /// as why, as a gone one is, in its place, its scope joining no other
    /// root's: one replaced at its path; one naming the project's directory
    /// by the identity of another, the project's root kept; and one whose
    /// directory differs from the one it named only by its birth time. A
    /// path whose directory is the one it named, by its real path, through
    /// a symlink, or the project's, is its root, and so is one through a
    /// symlink pointed since at another granted directory, which keeps its
    /// own grants alone: the root is the real path read at the grant.
    #[test]
    fn a_root_replaced_since_its_grant_is_held_as_why() {
        use crate::{Authority, FsGrant, Grants};

        let dir = crate::test_support::TestDir::on_disk("root_replaced");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "r", "w", "x", "y"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("w", at("to_w")).unwrap();
        std::os::unix::fs::symlink("w", at("still_w")).unwrap();
        let (r, to_w, still_w, x) = (
            checked(&at("r")),
            checked(&at("to_w")),
            checked(&at("still_w")),
            checked(&at("x")),
        );
        let p_as_x = (
            at("p"),
            crate::GrantedDir {
                real: at("p"),
                id: x.1.id,
            },
        );
        // a birth time no directory made since has
        let y = checked(&at("y"));
        let y_reborn = (
            y.0,
            crate::GrantedDir {
                id: crate::file_id::FileId {
                    born: Some(std::time::UNIX_EPOCH),
                    ..y.1.id
                },
                ..y.1
            },
        );
        // the swaps another process makes before the first import
        std::fs::rename(at("r"), at("r_before")).unwrap();
        std::fs::create_dir(at("r")).unwrap();
        std::fs::remove_file(at("to_w")).unwrap();
        std::os::unix::fs::symlink("x", at("to_w")).unwrap();
        let authority = Authority::Confined(Grants {
            fs: Some(FsGrant {
                read: [r, x, p_as_x, y_reborn].into(),
                write: [to_w, still_w, checked(&at("p"))].into(),
            }),
            ..Grants::default()
        });
        let table = root_table(Some(&at("p")), Some(&authority));
        let path = |name: &str| at(name).display().to_string();
        let shown = show(&table);
        let replaced = |name: &str| {
            Some(format!(
                "the directory granted as {} was replaced since the run started",
                path(name)
            ))
        };
        assert_eq!(
            shown,
            [
                (Some(path("p")), vec![], Scope::ALL, None),
                (None, vec![path("p")], Scope::READ, replaced("p")),
                (None, vec![path("r")], Scope::READ, replaced("r")),
                (Some(path("x")), vec![], Scope::READ, None),
                (None, vec![path("y")], Scope::READ, replaced("y")),
                (
                    Some(path("w")),
                    vec![path("still_w"), path("to_w")],
                    Scope::WRITE,
                    None
                ),
            ]
        );
    }

    /// A real path whose first granted path was refused, its directory
    /// replaced since, is opened again for the next path granted as it,
    /// which the directory there is the one it named: the refused root is
    /// held as why, and the reopened one is the later path's alone. Two
    /// paths name one real path by two directories only when the start's
    /// reads raced a swap, as `b`'s does here.
    #[test]
    fn a_real_path_is_opened_again_after_its_first_path_is_refused() {
        use crate::{Authority, FsGrant, Grants};

        let dir = crate::test_support::TestDir::on_disk("root_reopened");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "a", "c"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("a", at("b")).unwrap();
        // `b`'s real path read as `a`, and its identity `c`'s, a swap of the
        // two between the reads
        let b = (
            at("b"),
            crate::GrantedDir {
                real: at("a"),
                id: checked(&at("c")).1.id,
            },
        );
        let read = [checked(&at("a")), b].into();
        // the swap another process makes before the first import: `c`
        // moved to `a`, which is `b`'s directory alone
        std::fs::rename(at("a"), at("a_before")).unwrap();
        std::fs::rename(at("c"), at("a")).unwrap();
        let authority = Authority::Confined(Grants {
            fs: Some(FsGrant {
                read,
                write: std::collections::BTreeMap::new(),
            }),
            ..Grants::default()
        });
        let table = root_table(Some(&at("p")), Some(&authority));
        let path = |name: &str| at(name).display().to_string();
        let shown = show(&table);
        assert_eq!(
            shown,
            [
                (Some(path("p")), vec![], Scope::READ, None),
                (
                    None,
                    vec![path("a")],
                    Scope::READ,
                    Some(format!(
                        "the directory granted as {} was replaced since the run started",
                        path("a")
                    ))
                ),
                (Some(path("a")), vec![path("b")], Scope::READ, None),
            ]
        );
    }

    /// A root holds another's directory by its descriptor as well as by
    /// its real path's text: a directory beneath the root at the other's
    /// real path is held, its identity read as it opened or after; one at
    /// a path beneath the root that isn't there, as a swap between their
    /// opens leaves it, or that a symlink reaches, is held by none but
    /// itself; one that isn't open is held by the text alone; and a root
    /// that isn't open holds nothing.
    #[test]
    fn a_root_holds_a_directory_beneath_its_descriptor() {
        use super::{Joined, holds_beneath, opened};

        let dir = crate::test_support::TestDir::on_disk("root_beneath");
        let at = |name: &str| dir.path().join(name);
        for name in ["r/in", "r/w", "elsewhere"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("in", at("r/link")).unwrap();
        let joined = |real: &str, opened_at: &str, identify: bool| Joined {
            real: Some(at(real)),
            scope: Scope::READ,
            granted: Vec::new(),
            fd: opened(false, Some(&at(opened_at)), &[], identify),
        };
        let r = joined("r", "r", false);
        assert!(holds_beneath(&r, &r));
        for identify in [false, true] {
            assert!(holds_beneath(&r, &joined("r/in", "r/in", identify)));
            assert!(!holds_beneath(&r, &joined("r/w", "elsewhere", identify)));
            assert!(!holds_beneath(&r, &joined("r/link", "r/in", identify)));
        }
        assert!(holds_beneath(&r, &joined("r/gone", "r/gone", false)));
        assert!(!holds_beneath(
            &joined("r", "missing", false),
            &joined("r/in", "r/in", false)
        ));
        assert!(!holds_beneath(&joined("r/in", "r/in", false), &r));
    }

    /// A granted directory nests as it did when it was granted, by its real
    /// path then, which the descriptors confirm as the table is made: one
    /// moved since beneath a root, its symlink pointed along, is gone from
    /// its real path, taking nothing of that root's scope; one moved out
    /// from beneath a root since is gone too; and one beneath a root then
    /// and now joins its scope.
    #[test]
    fn a_root_nests_as_its_directory_did_when_granted() {
        use crate::{Authority, FsGrant, Grants};

        let dir = crate::test_support::TestDir::on_disk("root_nests_at_start");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "r/in", "r/out", "fake/w", "away"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("fake/w", at("wl")).unwrap();
        let authority = Authority::Confined(Grants {
            fs: Some(FsGrant {
                read: [checked(&at("r"))].into(),
                write: [
                    checked(&at("wl")),
                    checked(&at("r/in")),
                    checked(&at("r/out")),
                ]
                .into(),
            }),
            ..Grants::default()
        });
        // the moves another process makes before the first import: `w`
        // beneath `r`, `wl` pointed along, and `out` from beneath it
        std::fs::rename(at("fake/w"), at("r/w")).unwrap();
        std::fs::remove_file(at("wl")).unwrap();
        std::os::unix::fs::symlink("r/w", at("wl")).unwrap();
        std::fs::rename(at("r/out"), at("away/out")).unwrap();
        let table = root_table(Some(&at("p")), Some(&authority));
        let path = |name: &str| at(name).display().to_string();
        let shown = show(&table);
        let gone = |granted: &str, real: &str| {
            let what = if granted == real {
                path(granted)
            } else {
                format!("{}, {},", path(granted), path(real))
            };
            Some(format!(
                "the directory granted as {what} is gone, or no longer a directory"
            ))
        };
        assert_eq!(
            shown,
            [
                (Some(path("p")), vec![], Scope::READ, None),
                (Some(path("r")), vec![], Scope::READ, None),
                (Some(path("r/in")), vec![], Scope::ALL, None),
                (
                    Some(path("r/out")),
                    vec![],
                    Scope::ALL,
                    gone("r/out", "r/out")
                ),
                (
                    Some(path("fake/w")),
                    vec![path("wl")],
                    Scope::WRITE,
                    gone("wl", "fake/w")
                ),
            ]
        );
    }

    /// The table: the project's root first, read; each granted directory a
    /// root of its real path, the paths of one real path one root with its
    /// scopes joined and each spelling kept; each root reading and writing
    /// what every root holding it does, so write never implies read but a
    /// directory written inside a read one is read; a granted path gone
    /// before the table is made a root held as why; and unconfined, `/`,
    /// read and written, holding every other.
    #[test]
    fn the_root_table_joins_each_real_path_s_grants() {
        use std::path::PathBuf;

        use crate::{Authority, FsGrant, Grants};

        let dir = crate::test_support::TestDir::on_disk("root_table");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "p/out", "w", "r"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("w", at("to_w")).unwrap();
        let path = |name: &str| at(name).display().to_string();
        let grants = |read: &[PathBuf], write: &[PathBuf]| {
            Authority::Confined(Grants {
                fs: Some(FsGrant {
                    read: read.iter().map(|path| checked(path)).collect(),
                    write: write.iter().map(|path| checked(path)).collect(),
                }),
                ..Grants::default()
            })
        };
        let table = root_table(
            Some(&at("p")),
            Some(&grants(
                &[at("r"), at("p/./out"), at("missing")],
                &[at("p/out"), at("to_w"), at("w"), at("r/../w"), at("r")],
            )),
        );
        let shown: Vec<(Option<String>, Vec<String>, Scope, bool)> = show(&table)
            .into_iter()
            .map(|(path, granted, scope, why)| (path, granted, scope, why.is_none()))
            .collect();
        assert_eq!(
            shown,
            [
                (Some(path("p")), vec![], Scope::READ, true),
                (Some(path("missing")), vec![], Scope::READ, false),
                (Some(path("p/out")), vec![], Scope::ALL, true),
                (Some(path("r")), vec![], Scope::ALL, true),
                (Some(path("w")), vec![path("to_w")], Scope::WRITE, true),
            ]
        );
        assert!(
            table.roots()[1]
                .fd
                .as_ref()
                .is_err_and(|why| why.contains("is gone")),
            "{:?}",
            table.roots()[1].fd
        );
        let unconfined = root_table(Some(&at("p")), Some(&Authority::Unconfined));
        let shown: Vec<(Option<&str>, Scope, bool)> = unconfined
            .roots()
            .iter()
            .enumerate()
            .map(|(place, root)| {
                (
                    root.path(),
                    unconfined.project().scope_of(place),
                    root.whole(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (Some(path("p").as_str()), Scope::ALL, false),
                (Some("/"), Scope::ALL, true)
            ]
        );
    }

    /// A root that can't be opened is held as why, naming the path it was
    /// granted as and its real path, once when they're one path: gone when
    /// its real path holds nothing or no directory, else the OS's reason.
    #[test]
    fn a_root_that_cant_open_names_its_path_once() {
        use std::path::Path;

        use super::opened;

        let dir = crate::test_support::TestDir::on_disk("root_unopened");
        let file = dir.path().join("f");
        std::fs::write(&file, "").unwrap();
        let link = dir.path().join("to_f");
        std::os::unix::fs::symlink("f", &link).unwrap();
        let why = |project: bool, real: &Path, granted: &Path| {
            opened(project, Some(real), &[granted], true).unwrap_err()
        };
        let (f, l) = (file.display(), link.display());
        for (why, expected) in [
            (
                why(false, &file, &file),
                format!("the directory granted as {f} is gone, or no longer a directory"),
            ),
            (
                why(false, &file, &link),
                format!("the directory granted as {l}, {f}, is gone, or no longer a directory"),
            ),
            (
                why(true, &file, &file),
                format!("the project's directory, {f}, is gone, or no longer a directory"),
            ),
            (
                why(false, &dir.path().join("missing"), &link),
                format!(
                    "the directory granted as {l}, {}, is gone, or no longer a directory",
                    dir.path().join("missing").display()
                ),
            ),
        ] {
            assert_eq!(why, expected);
        }
        // a symlink on a real path, one swapped in since it was read
        let why = why(false, &link, &link);
        assert!(
            why.starts_with(&format!("the directory granted as {l} can't be opened: ")),
            "{why}"
        );
    }
}
