//! What a module graph has resolved: its admitted roots and the root and file
//! of each URL. Free of `deno_core` types.

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::path::{Path, PathBuf};

use url::Url;

use crate::resolver::{FileId, Resolution, ResolveErrorKind, Root, Roots};

/// What a graph has resolved: the roots it admitted, and the root and file
/// of each URL.
#[derive(Debug)]
pub(super) struct Graph {
    /// The project's root, and every package root a bare specifier resolved
    /// into.
    pub(super) roots: Roots,
    /// What each URL resolved to, the entry's included.
    pub(super) records: HashMap<Url, Record>,
    /// The recorded URLs by their real path, which `Path`'s ordering
    /// compares by component, so the paths under a directory are one range.
    /// A path has more than one URL when specifiers differ in their query or
    /// fragment.
    paths: BTreeMap<PathBuf, Vec<Url>>,
}

/// What a URL resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Record {
    /// The root it resolved inside.
    pub(super) root: Root,
    /// Its file's identity, the one loading it must find.
    pub(super) file_id: FileId,
}

impl Record {
    /// The record of `resolution`.
    fn new(resolution: &Resolution) -> Self {
        Self {
            root: Root {
                dir: resolution.root.clone(),
                origin: resolution.origin.clone(),
            },
            file_id: resolution.file_id,
        }
    }
}

impl Graph {
    /// The graph of `entry`, resolved with `roots`.
    pub(super) fn new(entry: &Resolution, roots: Roots) -> Self {
        let mut graph = Self {
            roots,
            records: HashMap::new(),
            paths: BTreeMap::new(),
        };
        graph.record(&entry.url, Record::new(entry));
        graph
    }

    /// Admits the root of `resolution` and records its URL inside it, with
    /// its file, changing nothing on an error.
    ///
    /// A new root around a module already recorded inside an outer one is
    /// a conflict, never first-wins: the module's origin would depend on
    /// which import ran first. A URL resolved again inside the same root is
    /// no change, since `deno_core` can resolve a specifier twice, and its
    /// first record stands: a load checks that file, and a URL already
    /// loaded is served from `deno_core`'s module map, never read again.
    /// Resolved again inside another root, it is a conflict.
    pub(super) fn admit(&mut self, resolution: &Resolution) -> Result<(), ResolveErrorKind> {
        let record = Record::new(resolution);
        let root = &record.root;
        let conflict =
            |path: PathBuf, recorded: &Root, admitted: &Root| ResolveErrorKind::RootConflict {
                path,
                recorded: Box::new(recorded.clone()),
                admitted: Box::new(admitted.clone()),
            };
        if self.roots.get(&root.dir).is_none()
            && let Some((path, recorded)) =
                self.recorded_under(&root.dir).find(|(path, recorded)| {
                    // one nested in the recorded root would now hold the module
                    root.dir != recorded.dir
                        && root.dir.starts_with(&recorded.dir)
                        && root.contains(path)
                })
        {
            return Err(conflict(path.to_owned(), recorded, root));
        }
        if let Some(recorded) = self.records.get(&resolution.url) {
            let path = || resolution.url.to_file_path().unwrap_or_default();
            if recorded.root.dir != root.dir {
                return Err(conflict(path(), &recorded.root, root));
            }
            // the first record stands
            return Ok(());
        }
        self.roots.admit(record.root.clone());
        self.record(&resolution.url, record);
        Ok(())
    }

    /// Records `url` as `record`, by its path too.
    fn record(&mut self, url: &Url, record: Record) {
        if let Ok(path) = url.to_file_path() {
            self.paths.entry(path).or_default().push(url.clone());
        }
        self.records.insert(url.clone(), record);
    }

    /// Every recorded real path under the directory `dir`, its own included,
    /// in path order, with the root of each of its records.
    fn recorded_under<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = (&'a Path, &'a Root)> {
        self.paths
            .range::<Path, _>((Bound::Included(dir), Bound::Unbounded))
            .take_while(move |(path, _)| path.starts_with(dir))
            .flat_map(|(path, urls)| {
                urls.iter()
                    .filter_map(|url| self.records.get(url))
                    .map(move |record| (path.as_path(), &record.root))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::Origin;

    fn package(dir: &str, name: &str) -> Root {
        Root {
            dir: PathBuf::from(dir),
            origin: Origin::Package {
                name: name.to_owned(),
                version: None,
                linked: true,
            },
        }
    }

    /// The resolution of the file at `path` inside `root`.
    fn resolution(path: &str, root: &Root) -> Resolution {
        Resolution {
            url: Url::from_file_path(path).unwrap(),
            file_id: FileId {
                dev: 1,
                ino: 1,
                born: None,
            },
            root: root.dir.clone(),
            origin: root.origin.clone(),
            files_consulted: Vec::new(),
        }
    }

    /// A graph of the project at `/a` whose entry is `/a/main.ts`, with
    /// `paths` recorded inside the project.
    fn project_graph(paths: &[&str]) -> Graph {
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut graph = Graph::new(
            &resolution("/a/main.ts", &project),
            Roots::new(project.dir.clone()),
        );
        for path in paths {
            graph.admit(&resolution(path, &project)).unwrap();
        }
        graph
    }

    fn conflict_path(result: Result<(), ResolveErrorKind>) -> Option<PathBuf> {
        match result {
            Err(ResolveErrorKind::RootConflict { path, .. }) => Some(path),
            Ok(()) => None,
            Err(error) => panic!("not a conflict: {error}"),
        }
    }

    /// A new root conflicts with a module recorded inside it by component:
    /// one under a sibling whose name extends the root's (`/a/b-c` beside
    /// `/a/b`) or under `node_modules` below it is no conflict.
    #[test]
    fn a_new_root_conflicts_only_with_what_it_would_hold() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&["/a/b-c/x.ts", "/a/b.ts", "/a/b/node_modules/n/x.mjs"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            None
        );
        let mut graph = project_graph(&["/a/b-c/x.ts", "/a/b/c/x.ts"]);
        let before = (graph.roots.clone(), graph.records.clone());
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/c/x.ts"))
        );
        // nothing changed on the error
        assert_eq!((graph.roots.clone(), graph.records.clone()), before);
        // the root's own directory, were it a module, is inside it
        let mut graph = project_graph(&["/a/b"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b"))
        );
    }

    /// The scan reads past a record the new root doesn't conflict with,
    /// one in a root nested in it, and the entry is recorded by its path.
    #[test]
    fn a_conflict_is_found_past_what_is_none_and_at_the_entry() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&["/a/b/z.ts"]);
        graph
            .admit(&resolution("/a/b/c/x.ts", &package("/a/b/c", "c")))
            .unwrap();
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/z.ts"))
        );
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut graph = Graph::new(
            &resolution("/a/b/main.ts", &project),
            Roots::new(project.dir.clone()),
        );
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/main.ts"))
        );
    }

    /// A root admitted already isn't checked again, and a module resolved
    /// again inside the same root keeps its first record; inside another
    /// root it is a conflict.
    #[test]
    fn a_re_resolve_keeps_the_first_record() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&[]);
        graph.admit(&resolution("/a/b/i.ts", &b)).unwrap();
        let mut again = resolution("/a/b/i.ts", &b);
        again.file_id.ino = 2;
        graph.admit(&again).unwrap();
        let url = Url::from_file_path("/a/b/i.ts").unwrap();
        assert_eq!(graph.records[&url].file_id.ino, 1);
        // the same URL inside an outer root, not admitted around it
        let a = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &a))),
            Some(PathBuf::from("/a/b/i.ts"))
        );
        // a query makes another URL for the same file, recorded on its own
        let mut query = resolution("/a/b/i.ts", &b);
        query.url.set_query(Some("q"));
        graph.admit(&query).unwrap();
        assert_eq!(graph.records.len(), 3);
        // a root nested in `b` around neither record is no conflict
        let x = package("/a/b/x", "x");
        graph.admit(&resolution("/a/b/x/y.ts", &x)).unwrap();
        // one around both conflicts with the first in path order
        let mut graph = project_graph(&["/a/b/c/z.ts", "/a/b/c/y.ts"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/c/y.ts"))
        );
    }
}
