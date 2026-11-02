//! The run's preparation: resolving the suite's files and writing a fresh copy
//! of each collection and project its workloads run in.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::collection::materialize;
use crate::error::BenchError;
use crate::project::materialize_project;
use crate::suite::Suite;

/// The collections a run copied: what each workload with a collection runs,
/// and what the run checks didn't change.
#[derive(Debug, Default)]
pub(super) struct Copies {
    /// Each copy's root, canonical.
    pub(super) roots: Vec<PathBuf>,
    /// Every file of every copy, entries included.
    pub(super) files: Vec<PathBuf>,
    /// The entry each workload with a collection runs, by workload name.
    pub(super) entries: BTreeMap<String, PathBuf>,
}

/// Copies each collection the suite's workloads name, once, from the pinned
/// tree into `<work_dir>/corpora/<tree>/<collection>`, and writes each of its
/// workloads' scripts (by their resolved paths in `scripts`) at its root
/// under the script's own file name, refusing one the collection already has.
pub(super) fn materialize_collections(
    suite: &Suite,
    corpora: Option<&(PathBuf, String)>,
    work_dir: &Path,
    scripts: &BTreeMap<&str, PathBuf>,
) -> Result<Copies, BenchError> {
    let mut copies = Copies::default();
    // each collection's copy root, by name, and the entries written
    let mut roots: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut entries: BTreeSet<PathBuf> = BTreeSet::new();
    for workload in &suite.workloads {
        let Some(collection) = workload.collection.as_deref() else {
            continue;
        };
        let Some((corpora_path, tree)) = corpora else {
            return Err(BenchError::SuiteInvalid(format!(
                "the workload `{}` names a collection, but the suite pins no `corpora`",
                workload.name
            )));
        };
        let root = if let Some(root) = roots.get(collection) {
            root.clone()
        } else {
            // canonical, so the paths written are the paths hashed and run
            fs::create_dir_all(work_dir).map_err(|source| BenchError::Write {
                path: work_dir.to_owned(),
                source,
            })?;
            let work_dir = work_dir.canonicalize().map_err(|source| BenchError::Read {
                path: work_dir.to_owned(),
                source,
            })?;
            // the tree id is git's own output, a hex id, matched to the pin
            let root = work_dir.join("corpora").join(tree).join(collection);
            copies
                .files
                .extend(materialize(corpora_path, tree, collection, &root)?);
            copies.roots.push(root.clone());
            roots.insert(collection, root.clone());
            root
        };
        // validation gives every collection workload's script a file name
        let file_name = workload.script.file_name().unwrap_or_default();
        let entry = root.join(file_name);
        if entries.insert(entry.clone()) {
            if fs::symlink_metadata(&entry).is_ok() {
                return Err(BenchError::EntryCollides {
                    workload: workload.name.clone(),
                    collection: collection.to_owned(),
                    entry: PathBuf::from(file_name),
                });
            }
            fs::copy(&scripts[workload.name.as_str()], &entry).map_err(|source| {
                BenchError::Write {
                    path: entry.clone(),
                    source,
                }
            })?;
            copies.files.push(entry.clone());
        }
        copies.entries.insert(workload.name.clone(), entry);
    }
    Ok(copies)
}

/// Copies each workload's project into `<work_dir>/projects/<workload>`
/// ([`materialize_project`]), adding the copies to `copies`.
pub(super) fn materialize_projects(
    suite: &Suite,
    dir: &Path,
    work_dir: &Path,
    copies: &mut Copies,
) -> Result<(), BenchError> {
    for workload in &suite.workloads {
        let Some(project) = &workload.project else {
            continue;
        };
        let copy = materialize_project(&workload.name, project, &workload.script, dir, work_dir)?;
        copies.roots.push(copy.root);
        copies.files.extend(copy.files);
        copies.entries.insert(workload.name.clone(), copy.entry);
    }
    Ok(())
}

/// Finds a file the suite names, canonicalized, relative to `dir`.
pub(super) fn resolve_file(dir: &Path, file: &Path) -> Result<PathBuf, BenchError> {
    let path = dir.join(file);
    path.canonicalize().map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            BenchError::SuiteFileMissing { path, source }
        } else {
            BenchError::Read { path, source }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_suite_file_is_an_input_error() {
        let dir = std::env::temp_dir();
        let error = resolve_file(&dir, Path::new("gro_bench_no_such_file.js")).unwrap_err();
        assert!(
            matches!(error, BenchError::SuiteFileMissing { .. }),
            "{error:?}"
        );
        assert!(error.is_input_error());
        // an existing file resolves
        assert!(resolve_file(Path::new("."), Path::new("Cargo.toml")).is_ok());
    }
}
