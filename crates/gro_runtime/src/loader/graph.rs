//! What a module graph has resolved: its admitted roots and the root and file
//! of each URL. Free of `deno_core` types.

use std::collections::HashMap;
use std::path::PathBuf;

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
        Self {
            roots,
            records: HashMap::from([(entry.url.clone(), Record::new(entry))]),
        }
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
        if self.roots.get(&root.dir).is_none() {
            for (url, Record { root: recorded, .. }) in &self.records {
                let Ok(path) = url.to_file_path() else {
                    continue;
                };
                // one nested in the recorded root would now hold the module
                if root.dir != recorded.dir
                    && root.dir.starts_with(&recorded.dir)
                    && root.contains(&path)
                {
                    return Err(conflict(path, recorded, root));
                }
            }
        }
        if let Some(recorded) = self.records.get(&resolution.url) {
            let path = || resolution.url.to_file_path().unwrap_or_default();
            if recorded.root.dir != root.dir {
                return Err(conflict(path(), &recorded.root, root));
            }
            // the first record stands
            return Ok(());
        }
        self.roots.admit(root.clone());
        self.records.insert(resolution.url.clone(), record);
        Ok(())
    }
}
