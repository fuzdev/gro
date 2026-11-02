//! The run's preparation: checking the corpora pin, resolving the suite's
//! files, writing a fresh copy of each collection and project its workloads
//! run in, and building the rows' commands.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::command::{Program, RowCommand, resolve_program, row_args};
use super::sample::Sampler;
use crate::collection::materialize;
use crate::error::BenchError;
use crate::git::read_corpora_tree;
use crate::phases::PHASES_FILE_ENV;
use crate::project::materialize_project;
use crate::suite::{FileArg, Suite};

/// Checks the suite's corpora checkout, when it pins one, against the pinned
/// tree, returning its resolved path and the tree.
pub(super) fn check_corpora_pin(
    suite: &Suite,
    dir: &Path,
) -> Result<Option<(PathBuf, String)>, BenchError> {
    suite
        .corpora
        .as_ref()
        .map(|pin| -> Result<_, BenchError> {
            let path = resolve_file(dir, &pin.path)?;
            let found = read_corpora_tree(&path)?;
            if found != pin.tree {
                return Err(BenchError::CorporaTree {
                    path,
                    pinned: pin.tree.clone(),
                    found,
                });
            }
            Ok((path, found))
        })
        .transpose()
}

/// What a run measures, resolved: each runtime's program, each row's
/// command, the copies its workloads run in, and the files it hashes.
pub(super) struct Plan {
    /// Each runtime's program, in the suite's runtime order.
    pub(super) programs: Vec<Program>,
    /// Each row's command, in [`Suite::rows`] order.
    pub(super) rows: Vec<RowCommand>,
    /// The copies of the collections and projects the workloads run in.
    pub(super) copies: Copies,
    /// Every file the run hashes: the programs, the suite's files, and every
    /// file of every copy.
    pub(super) artifacts: BTreeSet<PathBuf>,
    /// The scripts, file arguments, and stdin files, by their paths as the
    /// suite writes them.
    pub(super) suite_files: BTreeMap<String, PathBuf>,
}

/// Resolves the suite's files, recording each as an artifact and as a suite
/// file.
struct Resolver<'a> {
    dir: &'a Path,
    artifacts: BTreeSet<PathBuf>,
    suite_files: BTreeMap<String, PathBuf>,
}

impl Resolver<'_> {
    /// Resolves `file` ([`resolve_file`]) and records it.
    fn resolve(&mut self, file: &Path) -> Result<PathBuf, BenchError> {
        let resolved = resolve_file(self.dir, file)?;
        self.artifacts.insert(resolved.clone());
        self.suite_files
            .insert(file.to_string_lossy().into_owned(), resolved.clone());
        Ok(resolved)
    }
}

/// Resolves the suite's programs and files, writes the copies its workloads
/// run in, and builds each row's command, with a phases file from `sampler`
/// where it takes one.
pub(super) fn plan_rows(
    suite: &Suite,
    dir: &Path,
    work_dir: &Path,
    corpora: Option<&(PathBuf, String)>,
    sampler: &Sampler,
) -> Result<Plan, BenchError> {
    let programs = suite
        .runtimes
        .iter()
        .map(|runtime| resolve_program(runtime, dir))
        .collect::<Result<Vec<_>, _>>()?;
    let mut resolver = Resolver {
        dir,
        artifacts: programs
            .iter()
            .map(|program| program.path.clone())
            .collect(),
        suite_files: BTreeMap::new(),
    };
    // every workload's script
    let scripts = suite
        .workloads
        .iter()
        .map(|workload| Ok((workload.name.as_str(), resolver.resolve(&workload.script)?)))
        .collect::<Result<BTreeMap<_, _>, BenchError>>()?;
    // each runtime's stdin file, hashed like a file argument
    let stdins = suite
        .runtimes
        .iter()
        .map(|runtime| {
            runtime
                .stdin
                .as_ref()
                .map(|FileArg { file }| resolver.resolve(file))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut copies = materialize_collections(suite, corpora, work_dir, &scripts)?;
    materialize_projects(suite, dir, work_dir, &mut copies)?;
    let program_of: BTreeMap<&str, (&Program, &Option<PathBuf>)> = suite
        .runtimes
        .iter()
        .map(|runtime| runtime.name.as_str())
        .zip(programs.iter().zip(&stdins))
        .collect();
    let mut rows = Vec::new();
    for (workload, runtime) in suite.rows() {
        let name = workload.name.as_str();
        let script = copies.entries.get(name).unwrap_or_else(|| &scripts[name]);
        let (args, mut env) = row_args(runtime, workload, script, &mut |file: &Path| {
            resolver.resolve(file)
        })?;
        let (program, stdin) = program_of[runtime.name.as_str()];
        let phases_file = sampler.phases_file(&runtime.name);
        if let Some(file) = phases_file {
            env.insert(PHASES_FILE_ENV.to_owned(), file.display().to_string());
        }
        rows.push(RowCommand {
            runtime: runtime.name.clone(),
            workload: workload.name.clone(),
            program: program.clone(),
            args,
            env,
            warmup: workload.warmup,
            stdin: stdin.clone(),
            stdout: runtime.stdout,
            phases: phases_file.is_some(),
        });
    }
    let Resolver {
        mut artifacts,
        suite_files,
        ..
    } = resolver;
    artifacts.extend(copies.files.iter().cloned());
    Ok(Plan {
        programs,
        rows,
        copies,
        artifacts,
        suite_files,
    })
}

/// The collections and projects a run copied: what each workload with one
/// runs, and what the run checks didn't change.
#[derive(Debug, Default)]
pub(super) struct Copies {
    /// Each copy's root, canonical.
    pub(super) roots: Vec<PathBuf>,
    /// Every file of every copy, entries included.
    pub(super) files: Vec<PathBuf>,
    /// The entry each workload with a collection or a project runs, by
    /// workload name.
    pub(super) entries: BTreeMap<String, PathBuf>,
}

/// Copies each collection the suite's workloads name, once, from the pinned
/// tree into `<work_dir>/corpora/<tree>/<collection>`, and writes each of its
/// workloads' scripts (by their resolved paths in `scripts`) at its root
/// under the script's own file name, refusing one the collection already has.
fn materialize_collections(
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
        // `Suite::validate` refuses this before any run gets here; kept as
        // the backstop, since the copy can't be made without a pin
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
fn materialize_projects(
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
