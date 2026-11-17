//! `fs.open_dir` and `fs.real_path` take an absolute path and walk it as
//! Node's `realpath` does, a `..` after the symlink before it ([`walk`]):
//! the names before a `..` open as a directory beneath the longest root
//! that holds them by their text, the `..` takes a name off that
//! directory's real path, and what's left walks on, so a lookup always
//! starts at a root, and a `..` above one is text. A step that leaves the
//! longest root by a symlink is opened beneath the next longest that holds
//! its names, if one does, so a root granted inside another takes nothing
//! from what the outer one reached. `fs.real_path` takes only the roots the
//! run reads, `fs.open_dir` every root. The entry's real path is always
//! beneath its root, checked by its text before it's looked up, beneath
//! the root: one that left the root while it was opened is `io_error`, its
//! cause naming no path outside. A path no root holds is `not_granted`,
//! refused whether or not anything is there, and so is one through a
//! symlink that leaves every root holding it by its target's text: an
//! absolute symlink, even to a path inside the root, or a relative one
//! whose target climbs above the root, even back into it, both of which
//! Node follows; but for a root at `/`, an unconfined run's or a flag's
//! naming `/`, which resolves an absolute path as the kernel does.
//! Such a refusal is recorded in the run's record of refusals as it's
//! thrown, with what would have allowed it ([`FsNeed`]), which fails
//! the run whatever the program does with the error: a grant to read; and
//! for an op a handle's scope lacks, a handle `fs.open_dir` makes on a root
//! that grants it, since a grant never widens the handle that refused it,
//! or, with no such root, a grant of what the scope lacked. A need names
//! its kind, never a directory: a path a program chose, shown back, can
//! name another directory than the one the refusal needed, through a
//! symlink, and the refusal itself names the path.

use super::roots::Root;
use super::{Access, Failure, Scope, beneath_of, join, kind_of, root_fd, sys};
use crate::FsNeed;

/// What lets through an op on `targets`, absolute paths, that a handle
/// refused for want of `access`: a handle `fs.open_dir` makes on a root
/// doing it that holds every target by its text, when one does, since a
/// grant wouldn't widen the handle that refused it; else the grant of it.
pub(super) fn scope_needed(roots: &[Root], targets: &[&str], access: Access) -> FsNeed {
    let need = access.scope();
    // each target's roots doing `access`
    let mut held = targets.iter().map(|target| {
        let names: Vec<String> = target
            .split('/')
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        find_roots(roots, &names, false, need)
            .into_iter()
            .map(|(index, ..)| index)
            .collect::<Vec<_>>()
    });
    let first = held.next().unwrap_or_default();
    let rest: Vec<Vec<usize>> = held.collect();
    if first
        .iter()
        .any(|index| rest.iter().all(|other| other.contains(index)))
    {
        FsNeed::OpenDir
    } else {
        access.grant()
    }
}

/// The entry at `path`, an absolute path inside a root whose scope meets
/// `need`, opened as `opened`, as [`walk`] reaches it, each step opened
/// beneath its root by `openat2`: its root's place, the root, and its real
/// path beneath the root.
pub(super) fn reach<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
    need: Scope,
) -> Result<(usize, &'r Root, String), Failure> {
    walk(roots, path, what, opened, need, |root, rel, step| {
        let fd = root_fd(root, path)?;
        let found =
            sys::open_in_root(fd, root.path.clone(), rel, step, root.whole).map_err(|error| {
                if sys::is_escape(&error) {
                    Failure::unreached(path)
                } else if matches!(step, sys::Opened::Directory)
                    && error.kind() == std::io::ErrorKind::NotADirectory
                {
                    // the Node module names no cause for a path that is no
                    // directory
                    Failure::Fs {
                        kind: "not_a_directory",
                        path: path.to_owned(),
                        cause: None,
                    }
                } else {
                    Failure::of(kind_of(&error), path, &error)
                }
            })?;
        beneath_of(&root.path, found).map_err(|why| Failure::Fs {
            kind: "io_error",
            path: path.to_owned(),
            cause: Some(why),
        })
    })
}

/// The entry at `path`, an absolute path, reached as Node's `realpath`
/// reaches it, `..` after the symlink before it: the names up to each `..`
/// that follows a name are opened as a directory beneath the root that
/// holds them by their text, each `..` takes a name off that directory's
/// real path, and the names after it walk on from what's left, until the
/// whole is opened as `opened`. `open` opens a path beneath a root as it's
/// told and answers its real path beneath the root. So a name before a `..`
/// must be a directory, a `..` above a root is taken off the text of a real
/// path, never looked up, and every lookup starts at a root that holds its
/// names by their text. Each step costs the depth of the path it opens,
/// never the length of what's left of the path.
///
/// Only the roots whose scope meets `need` hold a path. A step is opened
/// beneath the longest root that holds its names, and, when that open is
/// `not_granted` (it left the root by a symlink), beneath the next longest,
/// and so on, so a root granted inside another refuses nothing the other
/// reaches.
///
/// The root's place, the root, and the entry's real path beneath it; a
/// `TypeError` naming `what` for a path that isn't absolute, and
/// `not_granted` for one whose names no root holds at any step.
fn walk<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
    need: Scope,
    mut open: impl FnMut(&'r Root, String, sys::Opened) -> Result<String, Failure>,
) -> Result<(usize, &'r Root, String), Failure> {
    if !path.starts_with('/') {
        return Err(Failure::Type(format!(
            "{what} takes an absolute path: {path}"
        )));
    }
    let mut step = |names: &[String], dir: bool, step: sys::Opened| {
        for (index, root, rel) in find_roots(roots, names, dir, need) {
            match open(root, rel, step) {
                Ok(beneath) => return Ok((index, root, beneath)),
                // it left this root by a symlink, so the next may hold it
                Err(Failure::NotGranted { .. }) => {}
                Err(failure) => return Err(failure),
            }
        }
        Err(Failure::unreached(path))
    };
    // a trailing `/` or `/.` asks for a directory, as the kernel reads it
    let dir = path.ends_with('/') || path.ends_with("/.");
    // the names left to walk, the next one last
    let mut left: Vec<&str> = path
        .split('/')
        .rev()
        .filter(|name| !name.is_empty() && *name != ".")
        .collect();
    // the names walked: a real path's, then those after it, none a `..`
    let mut names: Vec<String> = Vec::new();
    while let Some(name) = left.pop() {
        if name != ".." {
            names.push(name.to_owned());
            continue;
        }
        // `/..` is `/`, as the kernel reads it
        if !names.is_empty() {
            let (_, root, beneath) = step(&names, false, sys::Opened::Directory)?;
            names = join(&root.path, &beneath)
                .split('/')
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
        names.pop();
        while left.last() == Some(&"..") {
            left.pop();
            names.pop();
        }
    }
    step(&names, dir, opened)
}

/// The roots of `roots` whose scope meets `need` that hold the path of
/// `names`, none of them `.` or `..`, by their text, by their real path or
/// a path they were granted as, the longest first, each with its place and
/// the path beneath it for the kernel to open: `.` for the root's own
/// directory, and a trailing `/` when `dir` asks for a directory, but for
/// the root's own, as a handle's isn't asked for.
fn find_roots<'r>(
    roots: &'r [Root],
    names: &[String],
    dir: bool,
    need: Scope,
) -> Vec<(usize, &'r Root, String)> {
    let mut found: Vec<(usize, &Root, usize)> = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        if !root.scope.meets(need) {
            continue;
        }
        let depth = root
            .spellings()
            .filter(|held| {
                names.len() >= held.len()
                    && names
                        .iter()
                        .zip(held)
                        .all(|(name, root_name)| name == root_name)
            })
            .map(|held| held.len())
            .max();
        if let Some(depth) = depth {
            found.push((index, root, depth));
        }
    }
    // stable, so of two the same length the earlier root comes first
    found.sort_by(|(.., a), (.., b)| b.cmp(a));
    found
        .into_iter()
        .map(|(index, root, depth)| {
            let beneath = names[depth..].join("/");
            let rel = if beneath.is_empty() {
                ".".to_owned()
            } else if dir {
                format!("{beneath}/")
            } else {
                beneath
            };
            (index, root, rel)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::shown;
    use super::{Access, Failure, Root, Scope, find_roots, scope_needed, sys, walk};
    use crate::FsNeed;

    /// A root at `path`, never opened, reading and writing.
    fn root(path: &str) -> Root {
        scoped(path, &[], Scope::ALL)
    }

    /// A root at `path`, never opened, granted as `granted` too, doing what
    /// `scope` says.
    fn scoped(path: &str, granted: &[&str], scope: Scope) -> Root {
        Root::unopened(path, granted, scope)
    }

    /// An absolute path walks as Node's `realpath` walks it, over a tree of
    /// roots `/p` and `/p/inner` where `link` is a symlink to `a/b`, `f` a
    /// file, `missing` nothing, and `out` a symlink out of its root: a `..`
    /// follows the symlink before it and needs a directory there, a `..`
    /// above a root is taken off the text and can come back into one, the
    /// longest root that holds a step's names opens it, and the kernel is
    /// never handed a `..`.
    #[test]
    fn a_path_walks_its_dot_dots_after_the_symlinks_before_them() {
        let roots = [root("/p"), root("/p/inner"), root("")];
        let walked = |path: &str, opened: sys::Opened| {
            let mut opens = Vec::new();
            let result = walk(
                &roots,
                path,
                "fs.open_dir",
                opened,
                Scope::ALL,
                |root, rel, step| {
                    opens.push(format!("{}:{rel}", root.path));
                    let names: Vec<&str> = rel.split('/').filter(|name| !name.is_empty()).collect();
                    assert!(!names.contains(&".."), "{path} handed the kernel {rel}");
                    let mut real: Vec<&str> = root
                        .path
                        .split('/')
                        .filter(|name| !name.is_empty())
                        .collect();
                    for (at, name) in names.iter().enumerate() {
                        let last = at + 1 == names.len();
                        match *name {
                            "." => {}
                            "link" => real.extend(["a", "b"]),
                            "missing" => {
                                return Err(Failure::Fs {
                                    kind: "not_found",
                                    path: path.to_owned(),
                                    cause: None,
                                });
                            }
                            "f" if !last
                                || rel.ends_with('/')
                                || matches!(step, sys::Opened::Directory) =>
                            {
                                return Err(Failure::Fs {
                                    kind: "not_a_directory",
                                    path: path.to_owned(),
                                    cause: None,
                                });
                            }
                            "out" => return Err(Failure::unreached(path)),
                            name => real.push(name),
                        }
                    }
                    let real = format!("/{}", real.join("/"));
                    let beneath = real
                        .strip_prefix(root.path.as_str())
                        .expect("the fake stays beneath its root");
                    Ok(beneath.trim_start_matches('/').to_owned())
                },
            );
            (
                result
                    .map(|(index, _, beneath)| (index, beneath))
                    .map_err(shown),
                opens,
            )
        };
        for (path, expected, opens) in [
            ("/p", (0, ""), &["/p:."][..]),
            ("/p/", (0, ""), &["/p:."]),
            ("/p/a/b", (0, "a/b"), &["/p:a/b"]),
            ("/p/a/./b/", (0, "a/b"), &["/p:a/b/"]),
            ("/p/a/..", (0, ""), &["/p:a", "/p:."]),
            ("/p/link/..", (0, "a"), &["/p:link", "/p:a"]),
            ("/p/link/../c/", (0, "a/c"), &["/p:link", "/p:a/c/"]),
            ("/p/a/b/../../c", (0, "c"), &["/p:a/b", "/p:c"]),
            ("/p/a/../link/..", (0, "a"), &["/p:a", "/p:link", "/p:a"]),
            ("/p/a/../../p/x", (0, "x"), &["/p:a", "/p:x"]),
            ("/p/inner/x", (1, "x"), &["/p/inner:x"]),
            ("/p/inner", (1, ""), &["/p/inner:."]),
            ("/p/inner/../x", (0, "x"), &["/p/inner:.", "/p:x"]),
            ("/../p/a", (0, "a"), &["/p:a"]),
            ("/p/f", (0, "f"), &["/p:f"]),
        ] {
            assert_eq!(
                walked(path, sys::Opened::Entry),
                (
                    Ok((expected.0, expected.1.to_owned())),
                    opens.iter().map(|open| (*open).to_owned()).collect()
                ),
                "{path}"
            );
        }
        for (path, kind) in [
            ("/", "not_granted"),
            ("/q", "not_granted"),
            ("/pp", "not_granted"),
            ("/pinner", "not_granted"),
            ("/p/..", "not_granted"),
            ("/q/../p/a", "not_granted"),
            ("/p/a/../../q", "not_granted"),
            ("/p/out/..", "not_granted"),
            ("/p/missing/..", "not_found"),
            ("/p/f/..", "not_a_directory"),
            ("/p/f/", "not_a_directory"),
        ] {
            assert_eq!(
                walked(path, sys::Opened::Entry).0,
                Err((kind.to_owned(), path.to_owned())),
                "{path}"
            );
        }
        // a step whose names no root holds is refused before anything opens
        assert_eq!(
            walked("/q/../p/a", sys::Opened::Entry).1,
            Vec::<String>::new()
        );
        assert_eq!(
            walked("/p/f", sys::Opened::Directory).0,
            Err(("not_a_directory".to_owned(), "/p/f".to_owned()))
        );
        assert_eq!(
            walked("p/a", sys::Opened::Entry).0,
            Err((
                "TypeError".to_owned(),
                "fs.open_dir takes an absolute path: p/a".to_owned()
            ))
        );
    }

    /// A refused walk names the path as asked and needs a grant to read,
    /// whatever step it was refused at: a directory step's, the names
    /// before a `..` included, an entry's, or one that leaves the roots
    /// holding it through a symlink; a root that holds the names but
    /// doesn't do what's needed changes nothing.
    #[test]
    fn a_refused_walk_needs_a_grant_to_read() {
        let roots = [root("/p"), scoped("/w", &[], Scope::WRITE)];
        let needed = |path: &str, opened: sys::Opened, need: Scope| match walk(
            &roots,
            path,
            "fs.real_path",
            opened,
            need,
            |_, rel, _| match rel.as_str() {
                "out" => Err(Failure::unreached(path)),
                "." => Ok(String::new()),
                _ => Ok(rel),
            },
        ) {
            Err(Failure::NotGranted {
                path: refused,
                need,
            }) => {
                assert_eq!(refused, path, "the refusal names the path as asked");
                need
            }
            _ => panic!("{path} wasn't refused"),
        };
        let (directory, entry) = (sys::Opened::Directory, sys::Opened::Entry);
        for (path, opened, need) in [
            ("/q/a", directory, Scope::ALL),
            ("/q/a/", directory, Scope::ALL),
            ("/q/a", entry, Scope::READ),
            ("/q/../p/a", entry, Scope::READ),
            ("/usr/../etc", directory, Scope::ALL),
            ("/p/../etc", directory, Scope::ALL),
            ("/", entry, Scope::READ),
            ("/w/a/f.txt", entry, Scope::READ),
            ("/p/out", directory, Scope::ALL),
            ("/p/out", entry, Scope::READ),
        ] {
            assert_eq!(needed(path, opened, need), Some(FsNeed::Read), "{path}");
        }
    }

    /// An op a handle's scope lacks needs a handle `fs.open_dir` makes on
    /// a root doing it that holds every one of the op's targets by its
    /// text, by its real path or a path it was granted as; else a grant of
    /// what it lacks.
    #[test]
    fn a_scope_refusal_needs_a_handle_on_a_root_that_grants_it() {
        let roots = [
            scoped("/p", &[], Scope::READ),
            scoped("/p/out", &[], Scope::ALL),
            scoped("/p/out/deeper", &[], Scope::ALL),
            scoped("/data/w", &["/w"], Scope::WRITE),
            scoped("/data/w/sub", &[], Scope::ALL),
        ];
        let needed = |targets: &[&str], need: Scope| {
            let access = if need.write {
                Access::Write
            } else {
                Access::Read
            };
            scope_needed(&roots, targets, access)
        };
        let (open_dir, read, write) = (FsNeed::OpenDir, FsNeed::Read, FsNeed::Write);
        for (targets, need, expected) in [
            (&["/p/out/x"][..], Scope::WRITE, &open_dir),
            (&["/p/out"], Scope::WRITE, &open_dir),
            (&["/p/out/deeper/x"], Scope::WRITE, &open_dir),
            (&["/p/outer/x"], Scope::WRITE, &write),
            (&["/p/x"], Scope::WRITE, &write),
            (&["/w/x"], Scope::WRITE, &open_dir),
            (&["/data/w/sub/x"], Scope::READ, &open_dir),
            (&["/data/w/x"], Scope::READ, &read),
            (&["/p/x"], Scope::READ, &open_dir),
            // every target held by one root doing the need
            (&["/p/out/a", "/p/out/b"], Scope::WRITE, &open_dir),
            (&["/p/out/deeper/a", "/p/out/b"], Scope::WRITE, &open_dir),
            (&["/p/out/a", "/w/b"], Scope::WRITE, &write),
            (&["/p/src/a", "/p/out/b"], Scope::WRITE, &write),
            (&["/p/out/b", "/p/src/a"], Scope::WRITE, &write),
            (&[], Scope::WRITE, &write),
        ] {
            assert_eq!(&needed(targets, need), expected, "{targets:?}");
        }
    }

    /// A root holds a path by its real path or a path it was granted as,
    /// the longest first, each with the rest beneath it, and only when its
    /// scope meets the need: a root only written holds nothing for a read.
    #[test]
    fn roots_hold_a_path_by_any_spelling_the_longest_first() {
        let roots = [
            scoped("/p", &[], Scope::READ),
            scoped("/data/home/u/c", &["/home/u/c"], Scope::READ),
            scoped("/p/out", &[], Scope::WRITE),
            scoped("/p/out/both", &["/link"], Scope::ALL),
            scoped("", &["/gone"], Scope::READ),
        ];
        let held = |path: &str, need: Scope| {
            let names: Vec<String> = path
                .split('/')
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            find_roots(&roots, &names, false, need)
                .into_iter()
                .map(|(index, _, rel)| (index, rel))
                .collect::<Vec<_>>()
        };
        let owned = |found: &[(usize, &str)]| {
            found
                .iter()
                .map(|(index, rel)| (*index, (*rel).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(held("/p/a", Scope::ALL), owned(&[(0, "a")]));
        assert_eq!(held("/p", Scope::READ), owned(&[(0, ".")]));
        assert_eq!(
            held("/p/out/x", Scope::ALL),
            owned(&[(2, "x"), (0, "out/x")])
        );
        assert_eq!(held("/p/out/x", Scope::READ), owned(&[(0, "out/x")]));
        assert_eq!(held("/p/out/x", Scope::WRITE), owned(&[(2, "x")]));
        assert_eq!(
            held("/p/out/both/y", Scope::READ),
            owned(&[(3, "y"), (0, "out/both/y")])
        );
        assert_eq!(held("/link/y", Scope::WRITE), owned(&[(3, "y")]));
        assert_eq!(held("/home/u/c/d", Scope::READ), owned(&[(1, "d")]));
        assert_eq!(held("/data/home/u/c", Scope::READ), owned(&[(1, ".")]));
        assert_eq!(held("/home/u", Scope::READ), owned(&[]));
        assert_eq!(held("/home/u/cc", Scope::READ), owned(&[]));
        assert_eq!(held("/gone/x", Scope::READ), owned(&[(4, "x")]));
        assert_eq!(held("/", Scope::ALL), owned(&[]));
    }

    /// A step that leaves the longest root holding it by a symlink is opened
    /// beneath the next longest, so a root inside another refuses nothing
    /// the outer one reaches, and the entry is the outer root's; one that
    /// leaves every root is `not_granted`, and any other failure ends the
    /// walk at the first root.
    #[test]
    fn a_step_out_of_an_inner_root_falls_back_to_the_outer() {
        let roots = [root("/p"), root("/p/in")];
        let walked = |path: &str| {
            let mut opens = Vec::new();
            let result = walk(
                &roots,
                path,
                "fs.open_dir",
                sys::Opened::Entry,
                Scope::ALL,
                |root, rel, _| {
                    opens.push(format!("{}:{rel}", root.path));
                    match (root.path.as_str(), rel.as_str()) {
                        // `up`, a symlink to `../x`, leaves `/p/in` into
                        // `/p`, and `out` leaves both
                        ("/p/in", "up") | (_, "in/out" | "out") => Err(Failure::unreached(path)),
                        ("/p", "in/up") => Ok("x".to_owned()),
                        (_, "missing") => Err(Failure::Fs {
                            kind: "not_found",
                            path: path.to_owned(),
                            cause: None,
                        }),
                        (_, rel) => Ok(rel.to_owned()),
                    }
                },
            );
            (
                result
                    .map(|(index, _, beneath)| (index, beneath))
                    .map_err(shown),
                opens,
            )
        };
        assert_eq!(
            walked("/p/in/up"),
            (
                Ok((0, "x".to_owned())),
                vec!["/p/in:up".to_owned(), "/p:in/up".to_owned()]
            )
        );
        assert_eq!(
            walked("/p/in/out"),
            (
                Err(("not_granted".to_owned(), "/p/in/out".to_owned())),
                vec!["/p/in:out".to_owned(), "/p:in/out".to_owned()]
            )
        );
        assert_eq!(
            walked("/p/in/missing"),
            (
                Err(("not_found".to_owned(), "/p/in/missing".to_owned())),
                vec!["/p/in:missing".to_owned()]
            )
        );
    }
}
