//! An entry's import graph read before its run starts, and the loader's
//! reads served from it: what holds an entry in no project to the modules
//! its imports name.
//!
//! [`read_graph`] walks the graph from the entry before the run's box goes
//! up, on a thread with tsv's stack reserved (`crate::STACK_SIZE`): it
//! resolves each import as the loader does (`resolve.rs`), reads each module
//! once, by the identity its resolution found, never past the cap on a
//! module's source, and reads its imports in the same parse as its strip
//! (`strip_module_with_imports`), a JS module's by tsv's TypeScript grammar
//! (`js_module_imports`), keeping the bytes it read. It follows every static
//! import and re-export but one that is only types (`import type`, `export
//! type`, which the strip removes), one whose every specifier is a type
//! included (`import {type A} from './a.ts'`, which the strip keeps as
//! `import {} from './a.ts'`, so it still loads the module), and every
//! `import()` whose specifier is a string literal, in code that never runs
//! too; never a builtin or a module of Gro's library, which have no file,
//! nor an `import()` of a name built at run time, which it can't know. It
//! also keeps each `package.json` the resolutions read, the type markers
//! above the entry among them. A failed load, a strip's refusal or a module
//! too large, is kept as the load's refusal, which the loader then gives as
//! today's; one that didn't resolve is resolved again in the run.
//!
//! Its reads reach no further than the run's box would read them: the
//! entry's directory, beneath it, each type marker above it, only itself,
//! and each directory `--allow-read=` names, beneath it (`boxed.rs`). The
//! resolver's containment alone reaches further, since it admits a project
//! nested in that directory, whose packages resolve through its
//! `node_modules`, any of them a symlink to anywhere; a read past the reach
//! is refused ([`ReadRefusal::OutsideBox`]), a package's at its
//! `package.json`, a module's kept as its load's refusal
//! ([`LoadFailure::OutsideBox`]). A lookup is never refused, as the box
//! leaves a path's metadata open.
//!
//! [`Served`] then answers the run's loader: a module the walk read, at the
//! identity it read, is served those bytes, never read again, so what V8
//! runs is what the walk parsed; one at another identity is refused
//! ([`LoadFailure::ChangedSinceGraph`]); and one it never read loads from
//! disk only beneath a directory `--allow-read=` names, refused otherwise
//! ([`LoadFailure::OutsideGraph`]), as is a `package.json` read the same
//! way ([`ResolveErrorKind::ReadOutsideGraph`]), but one past the walk's
//! reach, refused as the walk refused it ([`ResolveErrorKind::ReadOutsideBox`]),
//! which no flag naming the entry's directory lets through. A JS module tsv
//! couldn't parse has no known imports, so it loads only from such a
//! directory too ([`LoadFailure::ImportsUnread`]). Each refusal a flag
//! naming the entry's directory would let through records it
//! ([`needs_read_flag`]), for the embedder's hint. A lookup still reaches
//! the disk: Landlock leaves a path's metadata open, so a run that reads
//! nothing of the directory still resolves in it.
//!
//! Kinds: the walk's reach and what the loader serves and refuses here are
//! native checks, a boundary against the run's own code and the files
//! around its entry, which reach no file of the entry's directory but
//! through the loader, and no file outside it the box wouldn't read; the
//! kernel box, which holds no rule on an entry in no project's directory,
//! is the boundary against an exploit of the engine, which reads the
//! graph's sources in the process's memory and nothing else of that
//! directory. A file the walk parses differently from V8 fails to load,
//! never loads more.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

use deno_core::{ModuleSource, ModuleSpecifier, RequestedModuleType};
use tsv_ts::{ImportRecordKind, Imports, TypeOnly};
use url::Url;

use super::Refused;
use super::boxed::{BoxCheck, Boxed};
use super::graph::{FailureReason, Graph, LoadFailure, Loaded, Reach, Reached, Record, Recording};
use super::load::{
    Code, check_requested, decoded, load, module_source, read_module, strip_refused,
};
use super::resolve::resolve_into;
use super::strip::{js_module_imports, strip_module_with_imports};
use crate::confine::BoxRoots;
use crate::file_id::FileId;
use crate::resolver::{
    Conditions, DiskTree, Entry, ModuleFormat, ReadRefusal, Resolution, ResolveErrorKind, Roots,
    SourceTree,
};

/// An entry's import graph, read before its run started (`read_graph`):
/// each module by its real path, each `package.json` its resolutions read,
/// and the reach the reads were checked against.
#[derive(Debug)]
pub struct ReadGraph {
    /// Each module read, by its real path.
    modules: HashMap<PathBuf, ReadModule>,
    /// Each `package.json` read, by its real path.
    files: HashMap<PathBuf, ReadFile>,
    /// What the walk's reads reached: a read past it was refused.
    reach: BoxCheck,
}

#[cfg(test)]
impl ReadGraph {
    /// The real paths of the modules read, in no order.
    pub(crate) fn modules(&self) -> impl Iterator<Item = &Path> {
        self.modules.keys().map(PathBuf::as_path)
    }

    /// The real paths of the `package.json` files read, in no order.
    pub(crate) fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }
}

/// A module the walk read.
#[derive(Debug)]
struct ReadModule {
    /// The identity it was read at.
    id: FileId,
    /// Its code, or the refusal its load gives.
    read: Result<ReadCode, Refused>,
}

/// A module's code as the walk read it.
#[derive(Debug)]
struct ReadCode {
    /// Its format.
    format: ModuleFormat,
    /// What V8 runs.
    code: Code,
    /// The BLAKE3 hash of the bytes read, for a graph recording in full.
    hash: Option<blake3::Hash>,
}

/// A `package.json` the walk read.
#[derive(Debug)]
struct ReadFile {
    /// The identity it was read at.
    id: FileId,
    /// Its bytes.
    bytes: Vec<u8>,
}

/// Reads the import graph of `entry`, resolved with `roots`, its packages
/// matching `conditions`, as the [module doc](self) says, reading nothing
/// past the project's directory, the type markers above it, and `read`, the
/// real paths of the directories `--allow-read=` names, and hashing what it
/// reads when `recording` is full.
///
/// Its strips recurse on the caller's stack, which must hold tsv's contract,
/// as `run_main`'s `# Stack` says.
pub fn read_graph(
    entry: &Resolution,
    roots: Roots,
    conditions: Conditions,
    recording: Recording,
    read: &[PathBuf],
) -> ReadGraph {
    let reach = BoxCheck::new(Some(walk_reach(entry, &roots.project().dir, read)));
    let recorder = Recorder::default();
    let tree = Boxed {
        check: &reach,
        tree: &recorder,
    };
    // the entry's own walk for its root, the type markers above it among it
    for consulted in &entry.files_consulted {
        if let Ok(Some(Entry::File { path, id })) = tree.lookup(&consulted.path) {
            // a file that can't be read now is read again, and refused, in the run
            let _ = tree.read(&path, id);
        }
    }
    let mut graph = Graph::new(entry, roots, Recording::Off).with_conditions(conditions);
    let mut modules = HashMap::new();
    let mut queue = VecDeque::from([entry.url.clone()]);
    while let Some(url) = queue.pop_front() {
        let Some(Record::Module(module)) = graph.records().get(&url) else {
            continue;
        };
        let id = module.file_id;
        let Ok(path) = url.to_file_path() else {
            continue;
        };
        if modules.contains_key(&path) {
            continue;
        }
        let (read, imports) = read_one(&url, id, &reach, recording);
        modules.insert(path, ReadModule { id, read });
        for (specifier, reach) in imports {
            let resolved = resolve_into(
                &mut graph,
                None,
                &tree,
                &specifier,
                url.as_str(),
                Reached::Import(reach),
            );
            if let Ok(resolved) = resolved
                && resolved.scheme() == "file"
            {
                queue.push_back(resolved);
            }
        }
    }
    ReadGraph {
        modules,
        files: recorder.files.into_inner(),
        reach,
    }
}

/// What the walk of `entry`, whose project's directory is `root`, reads:
/// `root`, beneath it, each type marker above it, only itself, and the
/// directories `read` names, beneath each.
fn walk_reach(entry: &Resolution, root: &Path, read: &[PathBuf]) -> BoxRoots {
    let markers = entry
        .files_consulted
        .iter()
        .filter(|file| !file.path.starts_with(root))
        .map(|file| file.path.clone());
    BoxRoots::new(
        std::iter::once(root.to_owned())
            .chain(markers)
            .chain(read.iter().cloned())
            .collect(),
    )
}

/// Reads the module at `url`, the file `id` names, once `reach` admits its
/// path, returning its code, or the refusal its load gives, and the
/// specifiers it imports, each with how it's reached.
fn read_one(
    url: &Url,
    id: FileId,
    reach: &BoxCheck,
    recording: Recording,
) -> (Result<ReadCode, Refused>, Vec<(String, Reach)>) {
    let (format, bytes) = match read_module(url, id, |path| reach.check_read(path)) {
        Ok(read) => read,
        Err(refused) => return (Err(refused), Vec::new()),
    };
    let hash = recording.is_full().then(|| blake3::hash(&bytes));
    match format {
        ModuleFormat::TypeScript => {
            let source = decoded(bytes);
            match strip_module_with_imports(&source, url) {
                Ok((code, imports)) => (
                    Ok(ReadCode {
                        format,
                        code: Code::Stripped(code),
                        hash,
                    }),
                    specifiers(&imports, &source),
                ),
                Err(refusal) => (Err(strip_refused(refusal)), Vec::new()),
            }
        }
        ModuleFormat::JavaScript => {
            let imports = {
                let source = String::from_utf8_lossy(&bytes);
                js_module_imports(&source, url).map(|imports| specifiers(&imports, &source))
            };
            match imports {
                Ok(imports) => (
                    Ok(ReadCode {
                        format,
                        code: Code::Bytes(bytes),
                        hash,
                    }),
                    imports,
                ),
                Err(refusal) => {
                    let failure = LoadFailure::ImportsUnread {
                        failure: refusal.failure,
                        at: refusal.at,
                    };
                    let refused = Refused::new(FailureReason::Load(failure), refusal.message);
                    (Err(refused), Vec::new())
                }
            }
        }
    }
}

/// The specifiers `imports`, read from `source`, name that load a module,
/// each with how it's reached: a static import or a re-export but one only
/// of types, and an `import()` of a string literal.
fn specifiers(imports: &Imports, source: &str) -> Vec<(String, Reach)> {
    imports
        .records
        .iter()
        .filter(|record| record.type_only != TypeOnly::Statement)
        .filter_map(|record| {
            let reach = match record.kind {
                ImportRecordKind::Static | ImportRecordKind::ReExport => Reach::Static,
                ImportRecordKind::Dynamic => Reach::Dynamic,
                // `import x = require('y')`, which the strip refuses
                _ => return None,
            };
            let specifier = record.specifier_value(source)?;
            Some((Cow::into_owned(specifier), reach))
        })
        .collect()
}

/// The disk, each `package.json` read kept, a later read of one served its
/// first read's bytes.
#[derive(Default)]
struct Recorder {
    /// Each `package.json` read.
    files: RefCell<HashMap<PathBuf, ReadFile>>,
}

impl SourceTree for Recorder {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        DiskTree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        if let Some(file) = self.files.borrow().get(path) {
            if file.id != id {
                return Err(ReadRefusal::ReplacedSinceFirstRead.into());
            }
            return Ok(file.bytes.clone());
        }
        let bytes = DiskTree.read(path, id)?;
        self.files.borrow_mut().insert(
            path.to_owned(),
            ReadFile {
                id,
                bytes: bytes.clone(),
            },
        );
        Ok(bytes)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        DiskTree.is_world_writable(dir)
    }
}

/// A read graph as a run's loader serves it, with the directories it reads
/// from disk beside it: those `--allow-read=` names.
#[derive(Debug)]
pub(super) struct Served {
    /// The graph read before the run started.
    graph: ReadGraph,
    /// The directories read from disk, each a real path.
    disk: Vec<PathBuf>,
}

impl Served {
    /// `graph`, served, reading from disk beneath `disk`.
    pub(super) const fn new(graph: ReadGraph, disk: Vec<PathBuf>) -> Self {
        Self { graph, disk }
    }

    /// Whether the real path `path` lies beneath a directory read from disk.
    fn on_disk(&self, path: &Path) -> bool {
        self.disk.iter().any(|dir| path.starts_with(dir))
    }

    /// Loads the module at `specifier`, the file `file_id` names, as the
    /// [module doc](self) says, returning with it, when `recording` is full,
    /// what was read.
    pub(super) fn load(
        &self,
        specifier: &ModuleSpecifier,
        requested: &RequestedModuleType,
        file_id: FileId,
        recording: Recording,
    ) -> Result<(ModuleSource, Option<Loaded>), Refused> {
        check_requested(specifier, requested)?;
        let path = specifier.to_file_path().ok();
        let read = path.as_ref().and_then(|path| self.graph.modules.get(path));
        let on_disk = path.as_deref().is_some_and(|path| self.on_disk(path));
        if let Some(read) = read
            && read.id == file_id
        {
            match &read.read {
                Ok(code) => {
                    let loaded = recording
                        .is_full()
                        .then_some(code.hash)
                        .flatten()
                        .map(|hash| Loaded {
                            format: code.format,
                            hash,
                        });
                    return Ok((module_source(specifier, code.code.clone()), loaded));
                }
                // a module whose imports tsv couldn't read loads from disk there
                Err(refused)
                    if !(on_disk
                        && matches!(
                            *refused.reason,
                            FailureReason::Load(LoadFailure::ImportsUnread { .. })
                        )) =>
                {
                    return Err(refused.clone());
                }
                Err(_) => {}
            }
        }
        if on_disk {
            return load(specifier, requested, file_id, recording, |_| Ok(()));
        }
        Err(match read {
            Some(_) => changed_since_graph(specifier),
            None => outside_graph(specifier),
        })
    }
}

/// The refusal of the module at `specifier`, outside the read graph.
fn outside_graph(specifier: &ModuleSpecifier) -> Refused {
    Refused::new(
        FailureReason::Load(LoadFailure::OutsideGraph),
        format!(
            "cannot load {specifier}: the entry is in no project, so the run loads only the \
             modules its imports name by a string literal, read before it started, and this \
             isn't one. Fix: add --allow-read= naming its directory"
        ),
    )
}

/// The refusal of the module at `specifier`, read before the run started at
/// another identity.
fn changed_since_graph(specifier: &ModuleSpecifier) -> Refused {
    Refused::new(
        FailureReason::Load(LoadFailure::ChangedSinceGraph),
        format!(
            "cannot load {specifier}: the file at its path isn't the one the run read before it \
             started, so it isn't loaded"
        ),
    )
}

/// Whether `reason` refuses what a directory `--allow-read=` names would
/// let through, a read graph's refusal.
pub(super) const fn needs_read_flag(reason: &FailureReason) -> bool {
    matches!(
        reason,
        FailureReason::Load(LoadFailure::OutsideGraph | LoadFailure::ImportsUnread { .. })
            | FailureReason::Resolve(ResolveErrorKind::ReadOutsideGraph(_))
    )
}

/// A read graph's `package.json` files as a tree: each read before the run
/// started served those bytes at the identity read, any other read from
/// disk only beneath a directory read from disk, refused otherwise.
pub(super) struct ServedTree<'a> {
    /// The graph served.
    pub(super) served: &'a Served,
}

impl SourceTree for ServedTree<'_> {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        DiskTree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        let read = self.served.graph.files.get(path);
        if let Some(file) = read
            && file.id == id
        {
            return Ok(file.bytes.clone());
        }
        if self.served.on_disk(path) {
            return DiskTree.read(path, id);
        }
        if read.is_some() {
            return Err(ReadRefusal::ReplacedSinceFirstRead.into());
        }
        Err(self.outside(path))
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        DiskTree.is_world_writable(dir)
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        if self.served.graph.files.contains_key(path) || self.served.on_disk(path) {
            Ok(())
        } else {
            Err(self.outside(path))
        }
    }
}

impl ServedTree<'_> {
    /// The refusal of a read of `path`, a real path the walk didn't read
    /// and no directory read from disk holds: past the walk's reach as the
    /// walk refused it, else as outside the graph.
    fn outside(&self, path: &Path) -> io::Error {
        self.served
            .graph
            .reach
            .check_read(path)
            .err()
            .unwrap_or_else(|| ReadRefusal::OutsideGraph(path.to_owned()).into())
    }
}
