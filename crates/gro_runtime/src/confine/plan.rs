//! The run's box as a plan: the rules it opens, each a path, the rights it
//! grants, and why it's there, derived before anything is opened.
//!
//! [`BoxPlan::for_run`] is pure. It reads two values and nothing else:
//! [`BoxInputs`], what the run is (its root, what its closure and grant
//! flags grant, the programs it pinned), and [`BoxFacts`], what the disk
//! held when it was read once (the root's top-level entries, the system's
//! directories, the link closure, git's home config). The realm's fs roots
//! come from the same inputs by their own derivation: they are directories,
//! while the kernel's rules are narrower on the project and wider for the
//! tools, so a path the realm admits the box can refuse (a new top-level
//! entry, a dot-entry, `.git`).
//!
//! What every boxed run gets:
//!
//! - the project read, beneath its root; a `node_modules` that is a symlink
//!   read at its real path, and each root of the link closure, a package
//!   linked from outside the project, read by its real path, each with
//!   execute too when the run may start a program;
//! - `/usr` read and execute, `/etc` read, and `/bin`, `/sbin`, `/lib`,
//!   `/lib64`, and `/lib32` read and execute only where each is a real
//!   directory: one that is a symlink into `/usr` gets no rule and is
//!   recorded ([`Skip::Covered`]);
//! - `/dev/zero`, `/dev/urandom`, and `/dev/random` read, `/dev/null` read
//!   and written, opened beneath `/`;
//! - `/proc` read, since a program's own memory figures come from it
//!   (`process.memoryUsage()` throws without it): boxed code sees every
//!   process's command line and status the user can see, never the
//!   environment, memory, or working directory of one outside the box,
//!   which Landlock's ptrace scope refuses.
//!
//! A run that may start a program also gets each pinned program's real
//! file read and executed (for a `node_modules/.bin` script, the `node` it
//! runs under), `node_modules` read and executed, and a directory private
//! to the run, read, written, and executed, which `--allow-temp` alone
//! grants too. A run that may start git ([`BoxInputs::git`]) gets what git
//! reads from its home config and the project's git directories outside
//! the root, each by its real path (`git.rs`, which refuses what it can't
//! read). A root with no project's `package.json` at or above it
//! ([`BoxInputs::unbounded`]) gets each type marker above it read,
//! file-only, by its real path ([`BoxInputs::markers`]), and its own rule
//! is the run's loader's ([`RuleSource::UnboundedRoot`]), which no
//! program's box takes unless a flag names the root.
//!
//! A run whose closure grants a program of the grants table that writes
//! tool directories gets each its `write` names that the project's declared
//! dependencies call for ([`ToolDir`]), each written plain
//! ([`Rights::WRITE_PLAIN`]: no symlink, no FIFO, no entry moved in from
//! another directory), made before the box, those in `node_modules` only
//! where it's the project's own directory, never a linked one; each
//! program's own box takes only its own (`program.rs`). A run that writes the
//! project's root (`gen`, `format`, `sync`, or a write flag naming the
//! root) gets one write rule per top-level directory or regular file, but
//! `.git`, `node_modules`, every dot-entry, every symlink, every regular
//! file with more than one link, and every FIFO, socket, or device, each of
//! the last three recorded ([`Skip`]); a top-level entry made later gets
//! none. A run in check mode writes only its tool directories and its
//! private directory.
//!
//! Grant flags widen the box as they widen the realm: a read flag by real
//! path, a write flag read and written beneath `/` (one naming the root is
//! the per-entry set), a run-in flag read and executed, and `--allow-temp`
//! the private directory. `--allow-run-write=` widens the box alone, the
//! realm gaining nothing ([`BoxInputs::run_writes`]): each directory it
//! names for a program is read and written beneath `/`, a rule of its own
//! ([`RuleSource::RunWrite`]) never merged with another source's on the
//! same path, and one naming the root is the per-entry set. A directory
//! either write flag names is written plain where it is a tool directory
//! (any, made or not) or where a linked `node_modules` leads, lies in one,
//! or holds one (`BoxPlan::flag_write_rights`). Each program's
//! own box takes it for that program alone (`program.rs`). A flag naming an
//! excluded entry (`--allow-write=.git`, `--allow-run-write=git:.git`) is
//! honored, but below.
//!
//! Refused here, before anything is opened ([`PlanRefusal`]), each a
//! refusal the caller exits 2 for: `-A`; a write flag, or a directory
//! `--allow-run-write=` names, that holds the root, beneath which no rule
//! can leave `.git` out; where gro's own code writes the whole project (the
//! command's closure, or a write flag naming the root), a directory
//! `--allow-run-write=` names whose real path lies under a dot-entry or
//! `node_modules`, which no write flag names short of the root, since only
//! the box's per-entry rules keep the realm out of those and that rule would
//! open them to it; a
//! `node` that is a script, a version manager's shim, which can't read its
//! manager's directory inside the box, unless a run-in flag admits it;
//! for a run that may start git, git's files `git.rs` can't read
//! (a [`GitFault`]); and a `node_modules` symlink to anything but a
//! directory named `node_modules` outside the project, neither in it nor
//! holding it, so a committed link to `~/.ssh` or to `.git` gets no rule.
//! Another checkout's `node_modules` is followed, read, and executed for a
//! run that may start a program, and the plan carries a note naming where
//! it leads and which programs `--allow-run-write=` lets write in it
//! ([`BoxPlan::note`]). No tool directory is made there, since that
//! checkout's own unconfined commands run what a tool writes there, so a
//! program writes there only what that flag names for it, as anywhere
//! outside the project, and `--allow-write=` naming it lets only gro's own
//! code write there.
//!
//! Kinds: the plan is the input of a boundary, the kernel's rules, whose
//! adversary is the project's code and tests, the tools and their
//! children, and an engine exploit, and whose asset is everything the rules
//! leave out (the home directory, `.git`, `node_modules`, the dot-entries,
//! the root's entry set, the network beyond loopback). It is defeated by a
//! write root a later unconfined command runs, by mode and time changes, by
//! a hard link made outside the run, by a nested `.git` or `node_modules`
//! under a written entry, which a rule on that entry can't leave out, and
//! by a kernel bug. The refusals and the note are review signals the run's
//! code could print as well.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::git::GitFault;
use super::listed;
use super::program::{ProgramWrites, program_rights};
use super::rules::{Need, RealRights, Rights};

/// What a run is, as its box is derived from it: built from the verb's plan
/// and the run's one resolution of its pins and granted directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts of the run's closure and flags, each read on its own"
)]
pub struct BoxInputs {
    /// The project's root, a real path.
    pub root: PathBuf,
    /// Whether the root is the directory of an entry with no project's
    /// `package.json` at or above it, which the run reads for its loader
    /// alone: its rule is [`RuleSource::UnboundedRoot`], which a program's
    /// box takes only where the program's `--allow-run-write=` names the
    /// root, so a program reads it only as a grant flag names it.
    pub unbounded: bool,
    /// Whether the run is unconfined (`-A`), which no box holds.
    pub unconfined: bool,
    /// Whether the command's closure writes the project's root, its check
    /// mode's writes dropped.
    pub writes_root: bool,
    /// The programs of the grants table the closure grants that write tool
    /// directories, by the name the run grants each as
    /// (`node_modules/.bin/vitest`).
    pub tools: BTreeMap<String, ToolWrites>,
    /// Whether the run may start a program: the closure grants one, or a
    /// flag does.
    pub spawns: bool,
    /// The packages the project's `package.json` declares in
    /// `dependencies` or `devDependencies`, by name.
    pub declared: BTreeSet<String>,
    /// The file each program the run pinned executes.
    pub programs: Vec<ProgramFile>,
    /// Whether `--allow-run=` grants a program the closure doesn't, whose
    /// file every program's box reads and executes.
    pub flag_programs: bool,
    /// The directories the grant flags list, each resolved.
    pub flags: Vec<GrantedFlagDir>,
    /// The directories `--allow-run-write=` lists, by the program each is
    /// for, each resolved: the box's alone, never the realm's.
    pub run_writes: Vec<RunWriteGrant>,
    /// Whether `--allow-temp` grants the realm's temporary directories,
    /// which the run makes in its private directory.
    pub temp: bool,
    /// `HOME`, if it's set.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, if it's set.
    pub xdg_config_home: Option<PathBuf>,
    /// `GIT_CONFIG_GLOBAL`, if it's set.
    pub git_config_global: Option<PathBuf>,
    /// Whether the run may start git, whose home config and the project's
    /// git directories outside the root the box then reads: every run of a
    /// command of Gro's that may start a program, whose tools run git, and
    /// an `fgro run` whose `--allow-run=` names `git`.
    pub git: bool,
    /// The type markers above the root, each a `package.json` by its real
    /// path, read file-only: an entry with no project's `package.json` at
    /// or above it has its own directory as its root, and a `.js` module's
    /// type is read from the nearest marker above it as each module loads,
    /// after the box is up.
    pub markers: Vec<PathBuf>,
}

/// The file a pinned program executes, as the box grants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramFile {
    /// A native binary, or a `#!` script the kernel runs: its real path.
    Direct {
        /// Its real path.
        real: PathBuf,
    },
    /// A `node_modules/.bin` script, which runs under `node`: the `node`
    /// found, which is what executes.
    Node {
        /// Where `node` was found on `PATH`.
        found: PathBuf,
        /// Its real path.
        real: PathBuf,
        /// Whether it's a `#!` script (a version manager's shim) rather
        /// than the runtime itself.
        script: bool,
    },
}

/// One of the grant flags that list directories: the command line's
/// (`fgro run`'s and a command of Gro's alike) and the box's plan's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirFlag {
    /// `--allow-read`.
    Read,
    /// `--allow-write`.
    Write,
    /// `--allow-run-in`.
    RunIn,
}

impl DirFlag {
    /// The flag as it's spelled, `--allow-read`, `--allow-write`, or
    /// `--allow-run-in`.
    pub const fn spelled(self) -> &'static str {
        match self {
            Self::Read => "--allow-read",
            Self::Write => "--allow-write",
            Self::RunIn => "--allow-run-in",
        }
    }
}

impl std::fmt::Display for DirFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.spelled())
    }
}

/// A directory a grant flag lists, resolved as the run grants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedFlagDir {
    /// The flag.
    pub flag: DirFlag,
    /// The directory as written.
    pub dir: String,
    /// Its real path.
    pub real: PathBuf,
}

/// The directories `--allow-run-write=` lists for one program of the run,
/// resolved as the run grants them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWriteGrant {
    /// The program, by the name the run grants it as (its key:
    /// `node_modules/.bin/vitest` for a local tool of the grants table, the
    /// name `--allow-run=` lists for one a flag grants).
    pub program: String,
    /// Its directories, each with the flag's spelling.
    pub dirs: Vec<RunWriteDir>,
}

/// A directory `--allow-run-write=` lists, resolved as the run grants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWriteDir {
    /// The program the flag names, as written.
    pub named: String,
    /// The directory as written.
    pub dir: String,
    /// Its real path.
    pub real: PathBuf,
}

/// What the disk held when the plan's facts were read, once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxFacts {
    /// The root's top-level entries, sorted by name.
    pub entries: Vec<TopEntry>,
    /// The root's `node_modules`.
    pub node_modules: NodeModules,
    /// `/bin`, `/sbin`, `/lib`, `/lib64`, and `/lib32`, each that exists,
    /// with its kind, a symlink not followed.
    pub system: Vec<(PathBuf, EntryKind)>,
    /// The link closure's roots beyond the project and its `node_modules`:
    /// each a package linked from outside them, by its real path.
    pub linked: Vec<PathBuf>,
    /// What git reads from its home config that exists, each by its real
    /// path: the config files, the files they name, and the defaults of
    /// `core.excludesFile` and `core.attributesFile` (`git::home_files`).
    pub git_home: Vec<PathBuf>,
    /// The project's git directories outside its root, each by its real
    /// path: an ancestor's `.git`, or a linked worktree's git directory and
    /// common directory (`git::git_dirs`).
    pub git_dirs: Vec<PathBuf>,
    /// Why git's home config or directories can't be read for the box, if
    /// they can't.
    pub git_fault: Option<GitFault>,
}

/// A top-level entry of the project's root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopEntry {
    /// Its name.
    pub name: OsString,
    /// Its kind, a symlink not followed.
    pub kind: EntryKind,
}

/// An entry's kind, a symlink not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A directory.
    Directory,
    /// A regular file, with its link count.
    File {
        /// Its link count.
        links: u64,
    },
    /// A symlink.
    Symlink,
    /// Anything else: a FIFO, a socket, a device.
    Other,
}

/// The project's `node_modules`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NodeModules {
    /// None, or a symlink that leads nowhere.
    #[default]
    Absent,
    /// A directory.
    Directory,
    /// A symlink, followed to its real path, a directory.
    Link {
        /// Its real path.
        real: PathBuf,
        /// The tool directories in `node_modules` that are directories
        /// there, with no symlink on the path beneath it, so a flag can
        /// name each.
        present: ToolDirs,
        /// Whether a `package.json` sits beside it, so it's a checkout's,
        /// whose flags a note can print.
        checkout: bool,
    },
}

/// The run's box, planned: each rule as a path, its rights, and its
/// source, and what the plan left out and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts of the run and its plan, each read on its own"
)]
pub struct BoxPlan {
    /// The project's root, a real path.
    root: PathBuf,
    /// The rules, in the order they open.
    rules: Vec<PlannedRule>,
    /// Whether the run gets a directory private to it.
    private_temp: bool,
    /// Whether the run may start a program, whose box nests in the run's
    /// (`program::ProgramRules`).
    spawns: bool,
    /// What got no rule, and why.
    skipped: Vec<Skipped>,
    /// Where the project's `node_modules` leads, when it's a symlink.
    node_modules_link: Option<PathBuf>,
    /// The directories `--allow-run-write=` lists, by program, as the
    /// inputs held them: what a [`RuleSource::RunWrite`] indexes.
    run_writes: Vec<RunWriteGrant>,
    /// The programs of the grants table that write tool directories, as the
    /// inputs held them.
    tools: BTreeMap<String, ToolWrites>,
    /// Whether `--allow-run=` grants a program the closure doesn't, as the
    /// inputs held it.
    flag_programs: bool,
    /// The tool directories in `node_modules` the declared dependencies
    /// call for, which a tool writing one needs `--allow-run-write=` to
    /// name where a symlinked `node_modules` leads.
    declared_in_node_modules: ToolDirs,
    /// The tool directories in `node_modules` that are directories where a
    /// symlinked one leads.
    linked_present: ToolDirs,
    /// Whether a symlinked `node_modules` leads into a checkout, a
    /// `package.json` beside it.
    linked_checkout: bool,
}

/// A rule the plan opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRule {
    /// How it opens, and what it grants.
    pub opening: Opening,
    /// Whether its absence is allowed.
    pub need: Need,
    /// Why it's there.
    pub source: RuleSource,
}

impl PlannedRule {
    /// What it grants.
    pub fn rights(&self) -> Rights {
        match self.opening {
            Opening::Real { rights, .. } => rights.into(),
            Opening::Beneath { rights, .. } => rights,
        }
    }
}

/// How a planned rule opens (`rules::BoxRules`), and what it grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// By its absolute real path, reading and executing only.
    Real {
        /// The path.
        path: PathBuf,
        /// What it grants.
        rights: RealRights,
    },
    /// Beneath an anchor's descriptor, every symlink on the path refused:
    /// the only way to a rule that writes.
    Beneath {
        /// The directory it opens beneath.
        anchor: Anchor,
        /// Its path beneath the anchor, `.` for the anchor itself.
        path: PathBuf,
        /// What it grants.
        rights: Rights,
    },
}

impl Opening {
    /// Its path: real, or beneath its anchor.
    pub fn path(&self) -> &Path {
        match self {
            Self::Real { path, .. } | Self::Beneath { path, .. } => path,
        }
    }
}

/// A directory a rule opens beneath.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The project's root.
    Project,
    /// `/`.
    Slash,
    /// The run's private directory.
    PrivateTemp,
}

/// Why a planned rule is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSource {
    /// The project's tree, read.
    Project,
    /// The root of an entry in no project ([`BoxInputs::unbounded`]), read
    /// for the run's own loader.
    UnboundedRoot,
    /// A top-level entry of the project, written.
    Entry,
    /// A tool directory, written.
    ToolDir(ToolDir),
    /// The project's `node_modules`, or where it leads.
    NodeModules,
    /// A package linked from outside the project.
    Linked,
    /// The system's programs, libraries, and config.
    System,
    /// A device.
    Device,
    /// `/proc`.
    Proc,
    /// The `node` a `node_modules/.bin` script runs under.
    Node,
    /// A program the run pinned.
    Program,
    /// Git's home config.
    GitHome,
    /// The project's git directory outside its root.
    GitDir,
    /// A type marker above the root ([`BoxInputs::markers`]).
    Marker,
    /// The run's private directory.
    PrivateTemp,
    /// A grant flag.
    Flag(DirFlag),
    /// `--allow-run-write=`, for the program at this index of the plan's
    /// (`BoxPlan::run_write`).
    RunWrite(usize),
}

/// A directory a tool writes on its own.
///
/// It's a write root of each program of the grants table that names it in
/// its `write`, made before the box when the command's closure grants such
/// a program and the project declares the dependency that writes it. One in `node_modules` is made only where that's the project's own
/// directory, never where a linked one leads. Deserialized from its path, as
/// the TS table's `BuiltinToolDir` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize)]
pub enum ToolDir {
    /// `.svelte-kit`, all of what `svelte-kit sync` writes, which `vite
    /// build` reads.
    #[serde(rename = ".svelte-kit")]
    SvelteKit,
    /// `.svelte-kit/generated/dev`, the modules `@sveltejs/kit` generates
    /// for a dev server, which its plugin writes in any Vite server it's
    /// in.
    #[serde(rename = ".svelte-kit/generated/dev")]
    SvelteKitDev,
    /// `.svelte-kit/types`, the route types `@sveltejs/kit` generates.
    #[serde(rename = ".svelte-kit/types")]
    SvelteKitTypes,
    /// `node_modules/$app`, the tsconfig and types `@sveltejs/kit`
    /// generates.
    #[serde(rename = "node_modules/$app")]
    App,
    /// `node_modules/.vite-temp`, where Vite's config loader writes the
    /// bundled config it imports and removes.
    #[serde(rename = "node_modules/.vite-temp")]
    ViteTemp,
    /// `node_modules/.vite/vitest`, Vitest's own cache, its module cache
    /// among them where the run gives one ([`VITEST_MODULE_CACHE`]),
    /// beside the optimized dependencies a later `dev` loads, which are no
    /// tool's.
    #[serde(rename = "node_modules/.vite/vitest")]
    Vitest,
}

impl ToolDir {
    /// Every tool directory, in the order their rules open: one holding
    /// another first.
    pub const ALL: [Self; 6] = [
        Self::SvelteKit,
        Self::SvelteKitDev,
        Self::SvelteKitTypes,
        Self::App,
        Self::ViteTemp,
        Self::Vitest,
    ];

    /// Its path beneath the project's root.
    pub const fn path(self) -> &'static str {
        match self {
            Self::SvelteKit => ".svelte-kit",
            Self::SvelteKitDev => ".svelte-kit/generated/dev",
            Self::SvelteKitTypes => ".svelte-kit/types",
            Self::App => "node_modules/$app",
            Self::ViteTemp => "node_modules/.vite-temp",
            Self::Vitest => "node_modules/.vite/vitest",
        }
    }

    /// Whether it sits in `node_modules`.
    pub const fn in_node_modules(self) -> bool {
        matches!(self, Self::App | Self::ViteTemp | Self::Vitest)
    }

    /// Its path beneath `node_modules`, for one that sits there, else
    /// beneath the project's root.
    pub fn path_in_node_modules(self) -> &'static str {
        let path = self.path();
        path.strip_prefix("node_modules/").unwrap_or(path)
    }

    /// The packages any of which, declared, calls for it.
    pub const fn declared_by(self) -> &'static [&'static str] {
        match self {
            Self::SvelteKit | Self::SvelteKitDev | Self::SvelteKitTypes | Self::App => {
                &[SVELTEKIT_PACKAGE]
            }
            Self::ViteTemp => &VITE_PACKAGES,
            Self::Vitest => &[VITEST_PACKAGE],
        }
    }

    /// Its bit in a [`ToolDirs`].
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Where Vitest keeps its module cache, beneath the project's root.
///
/// It's given in a run whose Vitest box is the grants table's alone
/// ([`BoxPlan::vitest_module_cache`]): inside Vitest's own tool directory,
/// which only Vitest's box writes, and never Vitest's default,
/// `node_modules/.vitest-cache`, which an unconfined Vitest reads.
pub const VITEST_MODULE_CACHE: &str = "node_modules/.vite/vitest/boxed-module-cache";

/// A set of [`ToolDir`]s: the tool directories a program writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolDirs(u8);

impl ToolDirs {
    /// None.
    pub const EMPTY: Self = Self(0);

    /// Whether it holds `dir`.
    pub const fn contains(self, dir: ToolDir) -> bool {
        self.0 & dir.bit() != 0
    }

    /// Whether it holds none.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// It and `other`, joined.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Each it holds, in [`ToolDir::ALL`]'s order.
    pub fn iter(self) -> impl Iterator<Item = ToolDir> {
        ToolDir::ALL
            .into_iter()
            .filter(move |dir| self.contains(*dir))
    }
}

impl FromIterator<ToolDir> for ToolDirs {
    fn from_iter<I: IntoIterator<Item = ToolDir>>(dirs: I) -> Self {
        Self(dirs.into_iter().fold(0, |bits, dir| bits | dir.bit()))
    }
}

/// A program of the grants table that writes tool directories, as the run
/// grants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolWrites {
    /// Its name in the table, as the tool is called (`vitest`), which the
    /// run's messages name it by.
    pub name: String,
    /// The tool directories its `write` names.
    pub tool_dirs: ToolDirs,
}

/// What got no rule, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Its absolute path.
    pub path: PathBuf,
    /// Why.
    pub why: Skip,
}

/// Why a path got no rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// A top-level symlink, which a rule opened beneath the root refuses.
    Symlink,
    /// A top-level regular file with more than one link, whose other names
    /// may lie outside the project.
    Linked,
    /// A system directory that is a symlink into `/usr`, which its rule
    /// covers.
    Covered,
    /// A top-level FIFO, socket, or device, which a run writes through, not
    /// in.
    Other,
}

/// Why a run can't be boxed as it's asked for, decided before anything is
/// opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanRefusal {
    /// `-A`, which no box holds.
    #[error(
        "-A would run a command of Gro's unconfined, and the binary runs each in a kernel box: \
         the project's Node gro is the unconfined spelling"
    )]
    Unconfined,
    /// A write flag, or a directory `--allow-run-write=` lists, names a
    /// directory that holds the project's root.
    #[error(
        "{}{dir} names {}, which holds the project, and no kernel rule beneath it can leave the \
         project's .git and node_modules out: name directories inside the project or beside it",
        written_as(program.as_deref()),
        real.display()
    )]
    WriteAboveRoot {
        /// The program `--allow-run-write=` lists it for, as written, or
        /// none for `--allow-write=`.
        program: Option<String>,
        /// The directory as written.
        dir: String,
        /// Its real path.
        real: PathBuf,
    },
    /// Gro's own code writes the whole project, and a directory
    /// `--allow-run-write=` lists lies under a dot-entry or `node_modules`,
    /// which no write flag names short of the root: only the box's
    /// per-entry rules keep the realm out of those, so the program's rule
    /// would open it to gro's code as well. Its message names the command
    /// where the caller has one ([`PlanRefusal::message`]).
    #[error("{}", realm_writes_message(program, dir, cause, None, GRO_CODE))]
    RealmWritesProgramDir {
        /// The program, as written.
        program: String,
        /// The directory as written.
        dir: String,
        /// What has gro's own code write the whole project.
        cause: RootWriter,
    },
    /// The `node` found on `PATH` is a script, which can't read its
    /// manager's directory inside the box.
    #[error(
        "the node found on PATH, {}, is a script (a version manager's shim), which can't reach \
         its manager's directory inside the kernel box: put the real node first on PATH, or \
         grant the manager's directory with {}=<dir>",
        found.display(),
        DirFlag::RunIn
    )]
    ScriptNode {
        /// Where it was found.
        found: PathBuf,
    },
    /// Git's home config or the project's git directories can't be read
    /// for a run that may start git ([`BoxInputs::git`]).
    #[error(
        "the kernel box can't read git's files ({0}): it grants git what its config and the \
         project's .git name, so it refuses a run that may start git when it can't read them"
    )]
    Git(GitFault),
    /// `node_modules` is a symlink to anything but another checkout's
    /// `node_modules`: a directory named otherwise (`~/.ssh`), one inside
    /// the project (`.git`), or one that holds it, which a rule there would
    /// open to the run and make its tool directories in.
    #[error(
        "node_modules is a symlink to {}, which isn't a node_modules directory outside the \
         project, so the box won't follow it: link node_modules to another checkout's \
         node_modules",
        real.display()
    )]
    NodeModulesLink {
        /// Where it leads.
        real: PathBuf,
    },
}

/// What has gro's own code write the whole project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootWriter {
    /// The command's closure (`gen`, `format`, `sync`).
    Closure,
    /// `--allow-write=` naming the root, as written.
    Flag(String),
}

impl PlanRefusal {
    /// The refusal's message, naming `command` (`fgro gen`, as a message
    /// names the command line) where it names the command that writes the
    /// project, and `realm` as what runs in the run's realm (`gro's own
    /// code` for a command of Gro's, `the run's own code` for `fgro run`),
    /// else as its `Display` prints it.
    pub fn message(&self, command: Option<&str>, realm: &str) -> String {
        match self {
            Self::RealmWritesProgramDir {
                program,
                dir,
                cause,
            } => realm_writes_message(program, dir, cause, command, realm),
            _ => self.to_string(),
        }
    }
}

/// The flag a write refusal names, up to the directory: `--allow-write=`,
/// or `--allow-run-write=<program>:` for a program's.
fn written_as(program: Option<&str>) -> String {
    program.map_or_else(
        || format!("{}=", DirFlag::Write),
        |program| format!("--allow-run-write={program}:"),
    )
}

/// What a command of Gro's runs in its realm, as a message names it.
pub const GRO_CODE: &str = "gro's own code";

/// [`PlanRefusal::RealmWritesProgramDir`]'s message, naming `command` as
/// what writes the project when the closure is the cause, or `the command`
/// with none, and `realm` as what runs in the run's realm.
fn realm_writes_message(
    program: &str,
    dir: &str,
    cause: &RootWriter,
    command: Option<&str>,
    realm: &str,
) -> String {
    let since = match cause {
        RootWriter::Closure => format!("{} writes the project", command.unwrap_or("the command")),
        RootWriter::Flag(root) => {
            format!("{}={root} lets {realm} write the project", DirFlag::Write)
        }
    };
    format!(
        "--allow-run-write={program}:{dir} would let {realm} write {dir} as well, since \
         {since} and only the kernel box keeps it out of .git, node_modules, and the \
         dot-entries: add {}={dir} to grant it to both",
        DirFlag::Write
    )
}

/// The devices every boxed run reads.
const READ_DEVICES: [&str; 3] = ["/dev/zero", "/dev/urandom", "/dev/random"];

/// The package that declares the tool directories of `@sveltejs/kit`.
const SVELTEKIT_PACKAGE: &str = "@sveltejs/kit";

/// The packages that declare `node_modules/.vite-temp`.
const VITE_PACKAGES: [&str; 2] = ["vite", "vitest"];

/// The package that declares `node_modules/.vite/vitest`.
const VITEST_PACKAGE: &str = "vitest";

/// The tool directories in `node_modules` the project's declared
/// dependencies call for.
fn declared_in_node_modules(inputs: &BoxInputs) -> ToolDirs {
    ToolDir::ALL
        .into_iter()
        .filter(|dir| {
            dir.in_node_modules()
                && dir
                    .declared_by()
                    .iter()
                    .any(|name| inputs.declared.contains(*name))
        })
        .collect()
}

/// Whether `path` prints inside single quotes as a shell passes it and a
/// write flag lists it as one directory: printable ASCII only, so no
/// control or lookalike character, and no `'`, `\\`, or `,`.
fn is_quotable(path: &str) -> bool {
    path.bytes()
        .all(|b| (b' '..=b'~').contains(&b) && !matches!(b, b'\'' | b'\\' | b','))
}

impl BoxPlan {
    /// The box for the run `inputs` describes, from the disk as `facts`
    /// read it, as the [module doc](self) says.
    ///
    /// # Errors
    ///
    /// Returns the first [`PlanRefusal`]: `-A`, a write flag above the
    /// root, a script `node` no run-in flag admits, a `node_modules`
    /// symlink above the root.
    pub fn for_run(inputs: &BoxInputs, facts: &BoxFacts) -> Result<Self, PlanRefusal> {
        refuse(inputs, facts)?;
        let mut plan = Self {
            root: inputs.root.clone(),
            rules: Vec::new(),
            private_temp: inputs.spawns || inputs.temp,
            spawns: inputs.spawns,
            skipped: Vec::new(),
            node_modules_link: match &facts.node_modules {
                NodeModules::Link { real, .. } => Some(real.clone()),
                NodeModules::Absent | NodeModules::Directory => None,
            },
            run_writes: inputs.run_writes.clone(),
            tools: inputs.tools.clone(),
            flag_programs: inputs.flag_programs,
            declared_in_node_modules: declared_in_node_modules(inputs),
            linked_present: match &facts.node_modules {
                NodeModules::Link { present, .. } => *present,
                NodeModules::Absent | NodeModules::Directory => ToolDirs::EMPTY,
            },
            linked_checkout: matches!(facts.node_modules, NodeModules::Link { checkout: true, .. }),
        };
        let read_maybe_execute = if inputs.spawns {
            RealRights::ReadExecute
        } else {
            RealRights::Read
        };
        let root_source = if inputs.unbounded {
            RuleSource::UnboundedRoot
        } else {
            RuleSource::Project
        };
        plan.beneath(Anchor::Project, ".", Rights::READ, root_source);
        if let Some(real) = plan.node_modules_link.clone() {
            plan.real(
                &real,
                read_maybe_execute,
                Need::Required,
                RuleSource::NodeModules,
            );
        }
        for linked in &facts.linked {
            plan.real(
                linked,
                read_maybe_execute,
                Need::Optional,
                RuleSource::Linked,
            );
        }
        for marker in &inputs.markers {
            plan.real(marker, RealRights::Read, Need::Optional, RuleSource::Marker);
        }
        plan.system(facts);
        if inputs.writes_root || writes_root_by_flag(inputs) || run_writes_root(inputs) {
            plan.entries(facts);
        }
        if inputs.spawns {
            plan.spawning(inputs, facts);
        }
        if plan.private_temp {
            plan.beneath(
                Anchor::PrivateTemp,
                ".",
                Rights::READ | Rights::WRITE | Rights::EXECUTE,
                RuleSource::PrivateTemp,
            );
        }
        if !inputs.tools.is_empty() {
            plan.add_tool_dirs(inputs, facts);
        }
        plan.flags(inputs);
        plan.run_write_rules(inputs);
        Ok(plan)
    }

    /// The project's root, a real path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The rules, in the order they open.
    pub fn rules(&self) -> &[PlannedRule] {
        &self.rules
    }

    /// The tool directories, each with its rule, in the order they open.
    pub fn tool_dirs(&self) -> impl Iterator<Item = (ToolDir, &PlannedRule)> {
        self.rules.iter().filter_map(|rule| match rule.source {
            RuleSource::ToolDir(dir) => Some((dir, rule)),
            _ => None,
        })
    }

    /// The directories `--allow-run-write=` lists, by program, each at the
    /// index [`RuleSource::RunWrite`] names it by.
    pub fn run_writes(&self) -> &[RunWriteGrant] {
        &self.run_writes
    }

    /// The directories `--allow-run-write=` lists for the program at
    /// `index`, as [`RuleSource::RunWrite`] names it.
    pub fn run_write(&self, index: usize) -> Option<&RunWriteGrant> {
        self.run_writes.get(index)
    }

    /// Whether the run gets a directory private to it.
    pub const fn private_temp(&self) -> bool {
        self.private_temp
    }

    /// The programs of the grants table that write tool directories, by the
    /// name the run grants each as.
    pub const fn tools(&self) -> &BTreeMap<String, ToolWrites> {
        &self.tools
    }

    /// Where Vitest keeps its module cache in this run, an absolute path
    /// ([`VITEST_MODULE_CACHE`]), when Vitest's box is the grants table's
    /// alone: the run starts the program that writes
    /// `node_modules/.vite/vitest`, the run made that directory (the
    /// project's own `node_modules`, never where a linked one leads), and no
    /// grant flag widens that program's box: neither `--allow-read=` nor
    /// `--allow-run-in=`, which widen every box by a directory, nor
    /// `--allow-run=` naming a program the closure doesn't grant, which can
    /// widen every box by that program's file, nor `--allow-run-write=`
    /// naming it. `None` otherwise, which leaves the cache off.
    ///
    /// Kind: a guardrail in Gro's own code, which passes Vitest the path.
    /// Its adversary is code a test wrote into the cache, which every later
    /// run given the path executes in Vitest's box until the lockfile
    /// changes; it runs only inside Vitest's process, after Vitest's
    /// arguments are set, so it never reaches a run whose Vitest box a grant
    /// flag widens. It doesn't bound realm code, which can start Vitest with
    /// arguments of its own, nor an unconfined Vitest pointed at the
    /// directory.
    pub fn vitest_module_cache(&self) -> Option<PathBuf> {
        if !self.spawns || !self.tool_dirs().any(|(dir, _)| dir == ToolDir::Vitest) {
            return None;
        }
        let (program, _) = self
            .tools
            .iter()
            .find(|(_, tool)| tool.tool_dirs.contains(ToolDir::Vitest))?;
        let widened = self.flag_programs
            || self
                .run_writes
                .iter()
                .any(|grant| grant.program == *program)
            || self.rules.iter().any(|rule| {
                matches!(
                    rule.source,
                    RuleSource::Flag(DirFlag::Read | DirFlag::RunIn)
                )
            });
        (!widened).then(|| self.root.join(VITEST_MODULE_CACHE))
    }

    /// What each program of the run may write, every write set a program's
    /// box is built for ([`ProgramWrites`]): none, for a program with no
    /// write set of its own, and each of [`BoxPlan::writes_by_name`]; none
    /// at all for a run that starts no program. Every other program writes
    /// none, so a rule any of these takes is one some program's box may.
    pub fn program_writes(&self) -> Vec<ProgramWrites> {
        if !self.spawns {
            return Vec::new();
        }
        std::iter::once(ProgramWrites::NONE)
            .chain(self.writes_by_name().into_values())
            .collect()
    }

    /// The write set of each program that writes more than its temporary
    /// directory, by the name the run grants it as: each program of the
    /// grants table with the tool directories its `write` names, and each
    /// `--allow-run-write=` names, with the root's per-entry rules when one
    /// of its directories is the root; none for a run that starts no
    /// program.
    pub fn writes_by_name(&self) -> BTreeMap<String, ProgramWrites> {
        if !self.spawns {
            return BTreeMap::new();
        }
        let mut writes: BTreeMap<String, ProgramWrites> = self
            .tools
            .iter()
            .map(|(program, tool)| {
                (
                    program.clone(),
                    ProgramWrites {
                        tool_dirs: tool.tool_dirs,
                        ..ProgramWrites::NONE
                    },
                )
            })
            .collect();
        for (index, grant) in self.run_writes.iter().enumerate() {
            let named = writes
                .entry(grant.program.clone())
                .or_insert(ProgramWrites::NONE);
            named.root = grant.dirs.iter().any(|dir| dir.real == self.root);
            named.run_write = Some(index);
        }
        writes
    }

    /// The first rule some program's box of the run would take
    /// (`program::program_rights`, for any of [`BoxPlan::program_writes`])
    /// that is on `dir`, a real path, or on a directory holding it
    /// (`holds`), with its absolute path and source, the private
    /// directory's own rule aside: a run that starts a program and makes
    /// its private directory in `dir` would give that program all of it by
    /// such a rule, its siblings' temporary directories among them, so the
    /// verb refuses one before the box. A rule only gro's own code takes,
    /// a write flag's among them, holds nothing of a program's.
    pub fn holding(&self, dir: &Path) -> Option<(PathBuf, RuleSource)> {
        let programs = self.program_writes();
        self.rules.iter().find_map(|rule| {
            let path = match &rule.opening {
                Opening::Real { path, .. } => path.clone(),
                // the root's own rule names the root, never `root/.`
                Opening::Beneath {
                    anchor: Anchor::Project,
                    path,
                    ..
                } if path.as_os_str() == "." => self.root.clone(),
                Opening::Beneath {
                    anchor: Anchor::Project,
                    path,
                    ..
                } => self.root.join(path),
                Opening::Beneath {
                    anchor: Anchor::Slash,
                    path,
                    ..
                } => Path::new("/").join(path),
                Opening::Beneath {
                    anchor: Anchor::PrivateTemp,
                    ..
                } => return None,
            };
            (programs
                .iter()
                .any(|writes| program_rights(rule.source, rule.rights(), *writes).is_some())
                && holds(&path, dir))
            .then_some((path, rule.source))
        })
    }

    /// Whether the run may start a program, each in a box of its own
    /// nested in the run's (`program::ProgramRules`).
    pub const fn spawns(&self) -> bool {
        self.spawns
    }

    /// What got no rule, and why, in the order the plan met it.
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// Where the project's `node_modules` leads, when it's a symlink.
    pub fn node_modules_link(&self) -> Option<&Path> {
        self.node_modules_link.as_deref()
    }

    /// The note a run prints for what the plan followed: a `node_modules`
    /// that is a symlink, read where it leads, and executed and written by
    /// the tools of a run that may start a program, which write there only
    /// what `--allow-run-write=` names for each: those it names there, then
    /// the command again with the flags that let the rest of the command's
    /// tools write their directories there, each quoted for a shell, or how
    /// one writes there when it can name none. `command` renders the command
    /// line, `fgro` and its command word first, with the flags it's given,
    /// already quoted, after its own grant flags, the command's own args
    /// after them, so a paste can't put a flag where the command reads it
    /// as an arg; none when the command line has no command word.
    ///
    /// Kind: a review signal naming flags to paste, which widen what the
    /// tools write into another checkout whose own unconfined commands run
    /// it (adversary: a committed `node_modules` link; asset: that
    /// checkout), so it says that beside them.
    pub fn note(&self, command: impl FnOnce(&[String]) -> Option<String>) -> Option<String> {
        self.node_modules_link.as_ref().map(|real| {
            let link = format!("note: node_modules is a symlink to {}", real.display());
            if !self.spawns {
                return format!("{link}, which the box follows to read");
            }
            let writers = self.run_writers_in(real);
            let flags = self.linked_flags(real).and_then(|flags| command(&flags));
            let tools = match (writers.is_empty(), flags) {
                (true, None) => "no tool writes in it: --allow-run-write=<program>:<dirs> naming \
                                 where it leads lets that tool write there"
                    .to_owned(),
                (true, Some(flags)) => format!(
                    "no tool writes in it; that checkout's own commands run what a tool writes \
                     there, and this command's tools write there with: {flags}"
                ),
                (false, flags) => {
                    let verb = if writers.len() == 1 {
                        "writes"
                    } else {
                        "write"
                    };
                    let only = format!(
                        "of the tools only {} {verb} in it, where --allow-run-write= names",
                        listed(writers.iter().map(|writer| (*writer).to_owned()))
                    );
                    match flags {
                        None => only,
                        Some(flags) => format!(
                            "{only}; that checkout's own commands run what a tool writes there, \
                             and the rest of this command's tools write there with: {flags}"
                        ),
                    }
                }
            };
            format!("{link}, which the box follows to read and execute, and {tools}")
        })
    }

    /// The `--allow-run-write=` flags, each quoted for a shell, that let
    /// each of the command's tools no flag lets write in `real`, where a
    /// symlinked `node_modules` leads, write its tool directories there:
    /// each directory where it's there, else `real` itself, which a flag
    /// can name only once it exists. None when no such tool is left, when
    /// `real` isn't a checkout's, a `package.json` beside it, or when
    /// `real` isn't a path a shell passes as printed in single quotes and
    /// the flag lists as one directory ([`is_quotable`]).
    fn linked_flags(&self, real: &Path) -> Option<Vec<String>> {
        if !self.linked_checkout {
            return None;
        }
        let target = real.to_str().filter(|target| is_quotable(target))?;
        let writing = self.run_writing_in(real);
        let mut tools: BTreeMap<&str, ToolDirs> = BTreeMap::new();
        for (program, tool) in &self.tools {
            let dirs: ToolDirs = tool
                .tool_dirs
                .iter()
                .filter(|dir| self.declared_in_node_modules.contains(*dir))
                .collect();
            if !dirs.is_empty() && !writing.contains(program.as_str()) {
                let joined = tools.entry(tool.name.as_str()).or_insert(ToolDirs::EMPTY);
                *joined = joined.union(dirs);
            }
        }
        let flags: Vec<String> = tools
            .iter()
            .map(|(name, dirs)| {
                let dirs: Vec<String> = if dirs.iter().all(|dir| self.linked_present.contains(dir))
                {
                    dirs.iter()
                        .map(|dir| format!("{target}/{}", dir.path_in_node_modules()))
                        .collect()
                } else {
                    vec![target.to_owned()]
                };
                format!("'--allow-run-write={name}:{}'", dirs.join(","))
            })
            .collect();
        (!flags.is_empty()).then_some(flags)
    }

    /// The programs `--allow-run-write=` lets write in `dir`, a real path,
    /// by a directory it names there or one holding it, each once, by the
    /// first of its names in the flags' sorted order, as the walls note
    /// names it, sorted.
    fn run_writers_in<'a>(&'a self, dir: &'a Path) -> Vec<&'a str> {
        self.run_writes_in(dir)
            .filter_map(|grant| grant.dirs.first().map(|named| named.named.as_str()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The programs `--allow-run-write=` lets write in `dir`, a real path,
    /// by the name the run grants each as.
    fn run_writing_in<'a>(&'a self, dir: &'a Path) -> BTreeSet<&'a str> {
        self.run_writes_in(dir)
            .map(|grant| grant.program.as_str())
            .collect()
    }

    /// The grants of `--allow-run-write=` naming a directory in `dir`, a
    /// real path, or one holding it.
    fn run_writes_in<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = &'a RunWriteGrant> + 'a {
        self.run_writes.iter().filter(move |grant| {
            grant
                .dirs
                .iter()
                .any(|named| holds(&named.real, dir) || holds(dir, &named.real))
        })
    }

    /// Adds a rule opened by `path`, a real path.
    fn real(&mut self, path: &Path, rights: RealRights, need: Need, source: RuleSource) {
        self.rules.push(PlannedRule {
            opening: Opening::Real {
                path: path.to_owned(),
                rights,
            },
            need,
            source,
        });
    }

    /// Adds a required rule opened beneath `anchor`.
    fn beneath(
        &mut self,
        anchor: Anchor,
        path: impl Into<PathBuf>,
        rights: Rights,
        source: RuleSource,
    ) {
        self.rules.push(PlannedRule {
            opening: Opening::Beneath {
                anchor,
                path: path.into(),
                rights,
            },
            need: Need::Required,
            source,
        });
    }

    /// The system's directories and devices, and `/proc`.
    fn system(&mut self, facts: &BoxFacts) {
        self.real(
            Path::new("/usr"),
            RealRights::ReadExecute,
            Need::Required,
            RuleSource::System,
        );
        self.real(
            Path::new("/etc"),
            RealRights::Read,
            Need::Required,
            RuleSource::System,
        );
        for (path, kind) in &facts.system {
            match kind {
                EntryKind::Directory => self.real(
                    path,
                    RealRights::ReadExecute,
                    Need::Optional,
                    RuleSource::System,
                ),
                EntryKind::Symlink => self.skipped.push(Skipped {
                    path: path.clone(),
                    why: Skip::Covered,
                }),
                EntryKind::File { .. } | EntryKind::Other => {}
            }
        }
        for device in READ_DEVICES {
            self.real(
                Path::new(device),
                RealRights::Read,
                Need::Optional,
                RuleSource::Device,
            );
        }
        self.beneath(
            Anchor::Slash,
            "dev/null",
            Rights::READ | Rights::WRITE,
            RuleSource::Device,
        );
        self.real(
            Path::new("/proc"),
            RealRights::Read,
            Need::Required,
            RuleSource::Proc,
        );
    }

    /// A write rule per top-level directory or regular file but those the
    /// [module doc](self) leaves out.
    fn entries(&mut self, facts: &BoxFacts) {
        for entry in &facts.entries {
            if entry.name.as_encoded_bytes().starts_with(b".") || entry.name == "node_modules" {
                continue;
            }
            let path = self.root.join(&entry.name);
            match entry.kind {
                EntryKind::Directory | EntryKind::File { links: 1 } => {
                    self.beneath(
                        Anchor::Project,
                        PathBuf::from(&entry.name),
                        Rights::WRITE,
                        RuleSource::Entry,
                    );
                }
                EntryKind::File { .. } => self.skipped.push(Skipped {
                    path,
                    why: Skip::Linked,
                }),
                EntryKind::Symlink => self.skipped.push(Skipped {
                    path,
                    why: Skip::Symlink,
                }),
                EntryKind::Other => self.skipped.push(Skipped {
                    path,
                    why: Skip::Other,
                }),
            }
        }
    }

    /// What a run that may start a program needs: each pinned file,
    /// `node_modules`, and for one that may start git, git's home config and
    /// the project's git directories outside the root.
    fn spawning(&mut self, inputs: &BoxInputs, facts: &BoxFacts) {
        for program in &inputs.programs {
            let (path, source) = match program {
                ProgramFile::Direct { real } => (real, RuleSource::Program),
                ProgramFile::Node { real, .. } => (real, RuleSource::Node),
            };
            self.real(path, RealRights::ReadExecute, Need::Required, source);
        }
        if facts.node_modules == NodeModules::Directory {
            self.rules.push(PlannedRule {
                opening: Opening::Beneath {
                    anchor: Anchor::Project,
                    path: PathBuf::from("node_modules"),
                    rights: Rights::READ | Rights::EXECUTE,
                },
                need: Need::Optional,
                source: RuleSource::NodeModules,
            });
        }
        if inputs.git {
            for git in &facts.git_home {
                self.real(git, RealRights::Read, Need::Optional, RuleSource::GitHome);
            }
            for dir in &facts.git_dirs {
                self.real(dir, RealRights::Read, Need::Optional, RuleSource::GitDir);
            }
        }
    }

    /// The tool directories the closure's programs write that the declared
    /// dependencies call for.
    fn add_tool_dirs(&mut self, inputs: &BoxInputs, facts: &BoxFacts) {
        let written = inputs
            .tools
            .values()
            .fold(ToolDirs::EMPTY, |dirs, tool| dirs.union(tool.tool_dirs));
        for dir in written.iter() {
            if !dir
                .declared_by()
                .iter()
                .any(|name| inputs.declared.contains(*name))
            {
                continue;
            }
            // another checkout's, which a program writes only as
            // `--allow-run-write=` names it
            if dir.in_node_modules() && facts.node_modules != NodeModules::Directory {
                continue;
            }
            self.beneath(
                Anchor::Project,
                dir.path(),
                Rights::WRITE_PLAIN,
                RuleSource::ToolDir(dir),
            );
        }
    }

    /// The grant flags' directories but a write flag naming the root, which
    /// is the per-entry set.
    fn flags(&mut self, inputs: &BoxInputs) {
        for flag in &inputs.flags {
            let source = RuleSource::Flag(flag.flag);
            match flag.flag {
                DirFlag::Read => {
                    self.real(&flag.real, RealRights::Read, Need::Required, source);
                }
                DirFlag::RunIn => {
                    self.real(&flag.real, RealRights::ReadExecute, Need::Required, source);
                }
                DirFlag::Write if flag.real == inputs.root => {}
                DirFlag::Write => {
                    let rights = self.flag_write_rights(&flag.real);
                    self.beneath(Anchor::Slash, beneath_root(&flag.real), rights, source);
                }
            }
        }
    }
}

impl BoxPlan {
    /// The directories `--allow-run-write=` lists, read and written
    /// beneath `/` as [`BoxPlan::flag_write_rights`] says, each its own
    /// rule for its program, but one naming the root, which is the
    /// per-entry set.
    fn run_write_rules(&mut self, inputs: &BoxInputs) {
        for (index, grant) in inputs.run_writes.iter().enumerate() {
            for dir in &grant.dirs {
                if dir.real != inputs.root {
                    let rights = self.flag_write_rights(&dir.real);
                    self.beneath(
                        Anchor::Slash,
                        beneath_root(&dir.real),
                        rights,
                        RuleSource::RunWrite(index),
                    );
                }
            }
        }
    }

    /// What a write flag's directory `real`, a real path, grants, for
    /// `--allow-write=` and `--allow-run-write=` alike: read and written,
    /// plain ([`Rights::WRITE_PLAIN`]) where it touches a tool directory
    /// ([`BoxPlan::touches_tool_dirs`]), as the tool directories are, so
    /// neither flag gives gro's own code or a tool a symlink, a FIFO, or an
    /// entry moved in there, and whole elsewhere.
    fn flag_write_rights(&self, real: &Path) -> Rights {
        if self.touches_tool_dirs(real) {
            Rights::READ | Rights::WRITE_PLAIN
        } else {
            Rights::READ | Rights::WRITE
        }
    }

    /// Whether `real`, a real path, is a tool directory or where a linked
    /// `node_modules` leads, or lies in or holds one, by whole components
    /// (`holds`), so `.svelte-kit2` neither lies in nor holds
    /// `.svelte-kit`. Every tool directory's path counts, made by the plan
    /// or not, whatever the project declares, since a later command that
    /// declares it, or a tool that writes it anyway, writes there by name.
    fn touches_tool_dirs(&self, real: &Path) -> bool {
        ToolDir::ALL
            .iter()
            .map(|dir| self.root.join(dir.path()))
            .any(|dir| holds(&dir, real) || holds(real, &dir))
            || self
                .node_modules_link
                .as_deref()
                .is_some_and(|dir| holds(dir, real) || holds(real, dir))
    }
}

/// Whether the rule at the absolute path `rule` is on `dir` or on a
/// directory holding it, compared by components, so `/tmp` holds
/// `/tmp/x` and `/tmp` itself but not `/tmpx`, and a `.` (a rule beneath
/// its anchor's own directory) names that directory. Landlock grants
/// beneath a rule whatever its directory holds.
pub(super) fn holds(rule: &Path, dir: &Path) -> bool {
    dir.starts_with(rule)
}

/// Whether a write flag names the project's root.
fn writes_root_by_flag(inputs: &BoxInputs) -> bool {
    inputs
        .flags
        .iter()
        .any(|flag| flag.flag == DirFlag::Write && flag.real == inputs.root)
}

/// Whether a directory `--allow-run-write=` lists is the project's root.
fn run_writes_root(inputs: &BoxInputs) -> bool {
    run_write_dirs(inputs).any(|dir| dir.real == inputs.root)
}

/// Every directory `--allow-run-write=` lists, in the inputs' order.
fn run_write_dirs(inputs: &BoxInputs) -> impl Iterator<Item = &RunWriteDir> {
    inputs.run_writes.iter().flat_map(|grant| &grant.dirs)
}

/// Whether `real`, a real path, holds the project's `root` without being
/// it.
fn holds_root(real: &Path, root: &Path) -> bool {
    real != root && root.starts_with(real)
}

/// What has gro's own code write the whole project, if anything does: the
/// command's closure, or a write flag naming the root.
fn root_writer(inputs: &BoxInputs) -> Option<RootWriter> {
    if inputs.writes_root {
        return Some(RootWriter::Closure);
    }
    inputs
        .flags
        .iter()
        .find(|flag| flag.flag == DirFlag::Write && flag.real == inputs.root)
        .map(|flag| RootWriter::Flag(flag.dir.clone()))
}

/// Whether `real`, a real path, lies under a top-level dot-entry or
/// `node_modules` of the project at `root`: by its first component
/// beneath the root, the holes in the per-entry rules that are directories.
fn under_excluded_entry(real: &Path, root: &Path) -> bool {
    real.strip_prefix(root)
        .ok()
        .and_then(|beneath| beneath.components().next())
        .is_some_and(|first| {
            let name = first.as_os_str();
            name.as_encoded_bytes().starts_with(b".") || name == "node_modules"
        })
}

/// The refusals the [module doc](self) lists, the first met.
fn refuse(inputs: &BoxInputs, facts: &BoxFacts) -> Result<(), PlanRefusal> {
    if inputs.unconfined {
        return Err(PlanRefusal::Unconfined);
    }
    if let Some(flag) = inputs
        .flags
        .iter()
        .find(|flag| flag.flag == DirFlag::Write && holds_root(&flag.real, &inputs.root))
    {
        return Err(PlanRefusal::WriteAboveRoot {
            program: None,
            dir: flag.dir.clone(),
            real: flag.real.clone(),
        });
    }
    if let Some(dir) = run_write_dirs(inputs).find(|dir| holds_root(&dir.real, &inputs.root)) {
        return Err(PlanRefusal::WriteAboveRoot {
            program: Some(dir.named.clone()),
            dir: dir.dir.clone(),
            real: dir.real.clone(),
        });
    }
    if let Some(cause) = root_writer(inputs)
        && let Some(dir) = run_write_dirs(inputs).find(|dir| {
            under_excluded_entry(&dir.real, &inputs.root)
                && !inputs.flags.iter().any(|flag| {
                    flag.flag == DirFlag::Write
                        && flag.real != inputs.root
                        && dir.real.starts_with(&flag.real)
                })
        })
    {
        return Err(PlanRefusal::RealmWritesProgramDir {
            program: dir.named.clone(),
            dir: dir.dir.clone(),
            cause,
        });
    }
    if let NodeModules::Link { real, .. } = &facts.node_modules
        && !(real.file_name() == Some(OsStr::new("node_modules"))
            && !real.starts_with(&inputs.root)
            && !inputs.root.starts_with(real))
    {
        return Err(PlanRefusal::NodeModulesLink { real: real.clone() });
    }
    if inputs.git
        && let Some(fault) = &facts.git_fault
    {
        return Err(PlanRefusal::Git(fault.clone()));
    }
    let run_in = |real: &Path| {
        inputs
            .flags
            .iter()
            .any(|flag| flag.flag == DirFlag::RunIn && real.starts_with(&flag.real))
    };
    if let Some(found) = inputs.programs.iter().find_map(|program| match program {
        ProgramFile::Node {
            found,
            real,
            script: true,
        } if !run_in(real) => Some(found),
        _ => None,
    }) {
        return Err(PlanRefusal::ScriptNode {
            found: found.clone(),
        });
    }
    Ok(())
}

/// The absolute path `path` beneath `/`.
fn beneath_root(path: &Path) -> PathBuf {
    path.strip_prefix("/").unwrap_or(path).to_owned()
}
