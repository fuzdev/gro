//! The run's preparation: checking the corpora pin, resolving the suite's
//! files, writing a fresh copy of each collection and project its workloads
//! run in, and building the rows' commands.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::command::{Program, RowCommand, resolve_program, row_args};
use super::sample::Sampler;
use crate::collection::materialize;
use crate::error::BenchError;
use crate::git::{git_command, read_corpora_collection};
use crate::phases::PHASES_FILE_ENV;
use crate::project::{materialize_project, resolve_packages, write_node_modules};
use crate::suite::{FileArg, Suite, Tail};

/// The suite's corpora checkout, checked against its pin.
pub(super) struct Corpora {
    /// The checkout, resolved.
    pub(super) path: PathBuf,
    /// Each pinned collection's tree id, by name, as the checkout has it.
    pub(super) collections: BTreeMap<String, String>,
}

/// Checks each collection the suite's corpora pin names, when it pins any,
/// against its pinned tree, refusing the first one off its pin, missing, or
/// with local changes.
pub(super) fn check_corpora_pin(suite: &Suite, dir: &Path) -> Result<Option<Corpora>, BenchError> {
    let Some(pin) = &suite.corpora else {
        return Ok(None);
    };
    let path = resolve_file(dir, &pin.path)?;
    for (collection, pinned) in &pin.collections {
        let found = read_corpora_collection(&path, collection)?;
        if found != *pinned {
            return Err(BenchError::CorporaCollection {
                path,
                collection: collection.clone(),
                pinned: pinned.clone(),
                found,
            });
        }
    }
    Ok(Some(Corpora {
        path,
        collections: pin.collections.clone(),
    }))
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
    corpora: Option<&Corpora>,
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
        .filter_map(|workload| Some((workload.name.as_str(), workload.script.as_deref()?)))
        .map(|(name, script)| Ok((name, resolver.resolve(script)?)))
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
    let mut copies = materialize_collections(suite, corpora, dir, work_dir, &scripts)?;
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
        let tail = match workload.tail() {
            Some(Tail::Script(_)) => {
                let script = copies.entries.get(name).unwrap_or_else(|| &scripts[name]);
                vec![script.as_os_str().to_owned()]
            }
            Some(Tail::Args(args)) => args.iter().map(OsString::from).collect(),
            // `Suite::validate` refuses this before any run gets here
            None => {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{name}` names neither a script nor args, or both"
                )));
            }
        };
        let (args, mut env) = row_args(runtime, workload, tail, &mut |file: &Path| {
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
            cwd: copies.dirs.get(name).cloned(),
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
    /// workload name, for a workload of a script.
    pub(super) entries: BTreeMap<String, PathBuf>,
    /// The copy's root each workload of args runs from, by workload name.
    pub(super) dirs: BTreeMap<String, PathBuf>,
    /// Each package copied into a collection's `node_modules`, as
    /// `name@version` ([`installed_as`]).
    pub(super) packages: BTreeSet<String>,
}

/// The package `name` copied from `from`, as `name@version` by the version
/// its `package.json` names, or `name` alone when it names none or can't be
/// read.
fn installed_as(name: &str, from: &Path) -> String {
    let version = fs::read_to_string(from.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("version")?.as_str().map(str::to_owned));
    version.map_or_else(|| name.to_owned(), |version| format!("{name}@{version}"))
}

/// Copies each collection the suite's workloads name, once, from its pinned
/// tree into `<work_dir>/corpora/<collection>`, with the
/// `node_modules` its workloads name (relative to `dir`), and writes each of
/// its workloads' scripts (by their resolved paths in `scripts`) at its root
/// under the script's own file name, refusing one the collection already has;
/// a workload of args runs from its root.
fn materialize_collections(
    suite: &Suite,
    corpora: Option<&Corpora>,
    dir: &Path,
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
        let Some((corpora, tree)) = corpora.and_then(|c| Some((c, c.collections.get(collection)?)))
        else {
            return Err(BenchError::SuiteInvalid(format!(
                "the workload `{}` names the collection `{collection}`, which `corpora.collections` doesn't pin",
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
            // one segment per collection, short, since a module's
            // resolution costs a `readlink` per component of its path
            let root = work_dir.join("corpora").join(collection);
            // every package resolved first, so a missing one fails before
            // any write
            let node_modules = workload
                .node_modules
                .as_ref()
                .map(|written| {
                    resolve_packages(&workload.name, &written.packages, dir)
                        .map(|packages| (written.stub_packages, packages))
                })
                .transpose()?;
            copies
                .files
                .extend(materialize(&corpora.path, tree, collection, &root)?);
            // validation gives every workload naming the collection the
            // same `node_modules`
            if let Some((stub_packages, packages)) = node_modules {
                copies.files.extend(write_node_modules(
                    &workload.name,
                    &root,
                    stub_packages,
                    &packages,
                    &[],
                )?);
                copies
                    .packages
                    .extend(packages.iter().map(|(name, from)| installed_as(name, from)));
            }
            // validation gives every workload naming the collection the
            // same `git`
            if workload.git {
                commit_copy(&root)?;
            }
            copies.roots.push(root.clone());
            roots.insert(collection, root.clone());
            root
        };
        let Some(script) = &workload.script else {
            copies.dirs.insert(workload.name.clone(), root);
            continue;
        };
        // validation gives every collection workload's script a file name
        let file_name = script.file_name().unwrap_or_default();
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

/// Makes the copy at `root` a git repository of its own, its files committed
/// but its `node_modules`, which the repository's own exclude file ignores:
/// a fixed identity and date, no signing, no hooks, no template, and
/// neither the user's nor the system's git config nor any `GIT_*` variable
/// of the caller's, so the commit is the same on every machine.
fn commit_copy(root: &Path) -> Result<(), BenchError> {
    let run = |args: &[&str]| {
        let mut command = Command::new("git");
        // a caller's `GIT_DIR`, `GIT_INDEX_FILE`, or `GIT_AUTHOR_NAME` would
        // move the repository or the commit
        for (name, _) in std::env::vars_os() {
            if name.as_encoded_bytes().starts_with(b"GIT_") {
                command.env_remove(name);
            }
        }
        command
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=gro_bench",
                "-c",
                "user.email=gro_bench@localhost",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        let line = git_command(root, args);
        let status = command.status().map_err(|source| BenchError::Spawn {
            command: line.clone(),
            source,
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(BenchError::CommandFailed {
                command: line,
                status,
            })
        }
    };
    // a template's hooks and files differ by machine, and with none there's
    // no `info/`
    run(&["init", "--quiet", "--template="])?;
    let info = root.join(".git/info");
    fs::create_dir_all(&info).map_err(|source| BenchError::Write {
        path: info.clone(),
        source,
    })?;
    let exclude = info.join("exclude");
    fs::write(&exclude, "/node_modules/\n").map_err(|source| BenchError::Write {
        path: exclude,
        source,
    })?;
    run(&["add", "--all"])?;
    run(&["commit", "--quiet", "--message", "the collection's copy"])
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
        let copy = materialize_project(
            &workload.name,
            project,
            workload.script.as_deref(),
            dir,
            work_dir,
        )?;
        copies.files.extend(copy.files);
        match copy.entry {
            Some(entry) => copies.entries.insert(workload.name.clone(), entry),
            None => copies.dirs.insert(workload.name.clone(), copy.root.clone()),
        };
        copies.roots.push(copy.root);
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
    use crate::git::read_command;

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

    /// A copy made a git repository commits every file but its
    /// `node_modules`, which it ignores, so its workspace reads clean.
    #[test]
    fn a_committed_copy_reads_clean_but_its_node_modules() {
        let root =
            std::env::temp_dir().join(format!("gro_bench_commit_copy_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules/a")).unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("src/a.ts"), "a").unwrap();
        fs::write(root.join("node_modules/a/package.json"), "{}").unwrap();
        commit_copy(&root).unwrap();
        let git = |args: &[&str]| {
            let output = read_command(&root, args).output().unwrap();
            String::from_utf8(output.stdout).unwrap()
        };
        let status = git(&["status", "--porcelain", "--untracked-files=all"]);
        let files = git(&["ls-files"]);
        let commit = git(&["log", "--format=%an %ae %aI %cn %ce %cI"]);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(status, "");
        assert_eq!(files, "package.json\nsrc/a.ts\n");
        assert_eq!(
            commit,
            "gro_bench gro_bench@localhost 2000-01-01T00:00:00Z gro_bench \
             gro_bench@localhost 2000-01-01T00:00:00Z\n"
        );
    }
}
