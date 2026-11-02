//! The run's artifacts: hashing the files a run executes and reads, and
//! reading the checkouts it depends on, to refuse a run whose artifacts
//! changed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use super::prepare::resolve_file;
use crate::error::BenchError;
use crate::git::checkout_root;
use crate::record::CheckoutState;
use crate::suite::Suite;

pub(super) fn hash_artifacts(
    artifacts: &BTreeSet<PathBuf>,
) -> Result<BTreeMap<PathBuf, String>, BenchError> {
    artifacts
        .iter()
        .map(|path| Ok((path.clone(), hash_file(path)?)))
        .collect()
}

fn hash_file(path: &Path) -> Result<String, BenchError> {
    let read_error = |source| BenchError::Read {
        path: path.to_owned(),
        source,
    };
    let file = File::open(path).map_err(read_error)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).map_err(read_error)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// The artifacts whose hash differs between `before` and `after`, which hash
/// the same set of paths.
pub(super) fn changed_artifacts(
    before: &BTreeMap<PathBuf, String>,
    after: &BTreeMap<PathBuf, String>,
) -> Vec<PathBuf> {
    before
        .iter()
        .filter(|&(path, hash)| after.get(path) != Some(hash))
        .map(|(path, _)| path.clone())
        .collect()
}

/// Reads the state of each checkout outside the suite's own (the one holding
/// `dir`, if any) that the workloads read, by the path as the suite writes
/// it, beside the checkout's root: each project link's target, refused when
/// it's in no checkout, and the checkout holding a script outside the
/// suite's (a collection's script runs from its copy, which the run covers).
pub(super) fn read_checkouts(
    suite: &Suite,
    dir: &Path,
) -> Result<BTreeMap<String, (PathBuf, CheckoutState)>, BenchError> {
    let own = checkout_root(dir);
    let other = |root: &PathBuf| own.as_ref() != Some(root);
    let mut checkouts = BTreeMap::new();
    for workload in &suite.workloads {
        if let Some(project) = &workload.project {
            for (package, target) in &project.links {
                let path = dir.join(target);
                let root =
                    checkout_root(&path).ok_or_else(|| BenchError::ProjectLinkUnversioned {
                        workload: workload.name.clone(),
                        package: package.clone(),
                        path: path.clone(),
                    })?;
                if other(&root) {
                    let state = CheckoutState::read(&root)?;
                    checkouts.insert(target.to_string_lossy().into_owned(), (root, state));
                }
            }
        } else if workload.collection.is_none() {
            let script = resolve_file(dir, &workload.script)?;
            if let Some(root) = checkout_root(&script).filter(other) {
                let state = CheckoutState::read(&root)?;
                checkouts.insert(
                    workload.script.to_string_lossy().into_owned(),
                    (root, state),
                );
            }
        }
    }
    Ok(checkouts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_artifacts_names_each_change() {
        let hashes = |pairs: &[(&str, &str)]| -> BTreeMap<PathBuf, String> {
            pairs
                .iter()
                .map(|&(path, hash)| (PathBuf::from(path), hash.to_owned()))
                .collect()
        };
        let before = hashes(&[("/a", "1"), ("/b", "2"), ("/c", "3")]);
        assert!(changed_artifacts(&before, &before).is_empty());
        let after = hashes(&[("/a", "1"), ("/b", "9"), ("/c", "3")]);
        assert_eq!(changed_artifacts(&before, &after), [PathBuf::from("/b")]);
    }
}
