//! What the box's plan reads from the disk, once: [`BoxFacts::read`].
//!
//! The facts are paths and kinds, never handles: the opener opens each
//! rule again, by real path with the kernel's name checked or beneath an
//! anchor with every symlink refused (`rules.rs`), so a symlink swapped in
//! since (an entry made a link, a directory on a path moved for one) fails
//! the box rather than widening it. A change of kind or link count since
//! the read isn't seen, which only a process outside the run can make.
//! Reading them is a guardrail against a plan that names what isn't there;
//! the boundary is the opening.

use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use super::closure::link_closure;
use super::git::{self, GitDirs, GitFault, GitHomeInputs};
use super::plan::{BoxFacts, BoxInputs, EntryKind, NodeModules, ToolDir, ToolDirs, TopEntry};
use super::rules::open_real_dir;

/// The system directories the box reads and executes only where each is a
/// real directory, as a split `/usr` has them.
pub(super) const MERGED_DIRS: [&str; 5] = ["/bin", "/sbin", "/lib", "/lib64", "/lib32"];

impl BoxFacts {
    /// The facts the plan of the run `inputs` describes reads: the root's
    /// top-level entries, each a symlink not followed, with a regular
    /// file's link count; its `node_modules`, a symlink followed to its real
    /// path when that is a directory; the merged system directories' kinds;
    /// the link closure from the root and where `node_modules` leads, none
    /// for an entry in no project ([`BoxInputs::unbounded`]); and
    /// for a run that may start git ([`BoxInputs::git`]), what git reads
    /// outside the project (`git::home_files`, `git::git_dirs`), or
    /// why that can't be read.
    ///
    /// # Errors
    ///
    /// Returns the error when the root can't be listed, an entry's kind
    /// can't be read (but one gone since the listing, which is passed
    /// over), or the root can't be opened by its real path.
    pub fn read(inputs: &BoxInputs) -> io::Result<Self> {
        let root = &inputs.root;
        let entries = top_entries(root)?;
        let node_modules = match entries.iter().find(|entry| entry.name == "node_modules") {
            Some(TopEntry {
                kind: EntryKind::Directory,
                ..
            }) => NodeModules::Directory,
            Some(TopEntry {
                kind: EntryKind::Symlink,
                ..
            }) => std::fs::canonicalize(root.join("node_modules"))
                .ok()
                .filter(|real| real.is_dir())
                .map_or(NodeModules::Absent, |real| NodeModules::Link {
                    present: present_tool_dirs(&real),
                    checkout: beside_package_json(&real),
                    real,
                }),
            _ => NodeModules::Absent,
        };
        let system = MERGED_DIRS
            .iter()
            .filter_map(|dir| {
                let path = PathBuf::from(dir);
                std::fs::symlink_metadata(&path)
                    .ok()
                    .map(|metadata| (path, kind_of(&metadata)))
            })
            .collect();
        let mut seeds = vec![(root.clone(), open_real_dir(root)?)];
        if let NodeModules::Link { real, .. } = &node_modules
            && !inputs.unbounded
            && !real.starts_with(root)
            && let Ok(pinned) = open_real_dir(real)
        {
            seeds.push((real.clone(), pinned));
        }
        let (git_home, git_dirs, git_fault) = if inputs.git {
            read_git(inputs)
        } else {
            (Vec::new(), Vec::new(), None)
        };
        let seeded = seeds.len();
        // the run reads an entry in no project's modules, a project nested
        // in its directory and that project's packages included, before the
        // box goes up, and only beneath that directory or one
        // `--allow-read=` names, so its box reads no package
        let (roots, _handles) = if inputs.unbounded {
            (Vec::new(), Vec::new())
        } else {
            link_closure(seeds)
        };
        Ok(Self {
            entries,
            node_modules,
            system,
            linked: roots.into_iter().skip(seeded).collect(),
            git_home,
            git_dirs,
            git_fault,
        })
    }
}

/// The root's top-level entries, sorted by name, each a symlink not
/// followed; one gone since the listing is passed over.
fn top_entries(root: &Path) -> io::Result<Vec<TopEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        // `DirEntry::metadata` doesn't follow a symlink
        match entry.metadata() {
            Ok(metadata) => entries.push(TopEntry {
                name: entry.file_name(),
                kind: kind_of(&metadata),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// The kind `metadata`, of an entry not followed, says.
fn kind_of(metadata: &std::fs::Metadata) -> EntryKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File {
            links: metadata.nlink(),
        }
    } else {
        EntryKind::Other
    }
}

/// What git reads outside the project, its files and its directories, and
/// the first fault reading them.
fn read_git(inputs: &BoxInputs) -> (Vec<PathBuf>, Vec<PathBuf>, Option<GitFault>) {
    let home = inputs.home.as_deref().filter(|home| home.is_absolute());
    let (dirs, dirs_fault) = match git::git_dirs(&inputs.root, home) {
        Ok(dirs) => (dirs, None),
        Err(fault) => (
            GitDirs {
                dirs: Vec::new(),
                top: inputs.root.clone(),
            },
            Some(fault),
        ),
    };
    let files = git::home_files(&GitHomeInputs {
        top: &dirs.top,
        home,
        xdg_config_home: inputs
            .xdg_config_home
            .as_deref()
            .filter(|dir| dir.is_absolute()),
        config_global: inputs.git_config_global.as_deref(),
    });
    let fault = dirs_fault.or_else(|| files.as_ref().err().cloned());
    (files.unwrap_or_default(), dirs.dirs, fault)
}

/// The tool directories in `node_modules` that are directories in `real`,
/// where a symlinked one leads, with no symlink on the path beneath it
/// (`.vite` of `.vite/vitest` included): read only so a note can name each
/// to a flag, which needs it to exist and must grant nothing outside
/// `real`.
fn present_tool_dirs(real: &Path) -> ToolDirs {
    ToolDir::ALL
        .into_iter()
        .filter(|dir| {
            let path = real.join(dir.path_in_node_modules());
            dir.in_node_modules()
                && std::fs::canonicalize(&path)
                    .is_ok_and(|resolved| resolved == path && resolved.is_dir())
        })
        .collect()
}

/// Whether a `package.json` sits beside `real`, where a symlinked
/// `node_modules` leads, so it's a checkout's: read only so a note prints
/// flags to paste for a checkout's and not for one like a global npm
/// prefix's `lib/node_modules`.
fn beside_package_json(real: &Path) -> bool {
    real.parent()
        .is_some_and(|parent| parent.join("package.json").is_file())
}
