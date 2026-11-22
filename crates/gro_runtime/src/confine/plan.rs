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
//! runs under), `node_modules` read and executed, git's home config read
//! by its real paths, and a directory private to the run, read, written,
//! and executed, which `--allow-temp` alone grants too.
//!
//! A run whose closure grants a local tool gets the tool directories the
//! project's declared dependencies call for ([`ToolDir`]), each written,
//! made before the box. A run that writes the project's root (`gen`,
//! `format`, `sync`, or a write flag naming the root) gets one write rule
//! per top-level directory or regular file, but `.git`, `node_modules`,
//! every dot-entry, every symlink, every regular file with more than one
//! link, and every FIFO, socket, or device, each of the last three recorded
//! ([`Skip`]); a top-level entry made later gets none. A run in check mode
//! writes only its tool directories and its private directory.
//!
//! Grant flags widen the box as they widen the realm: a read flag by real
//! path, a write flag read and written beneath `/` (one naming the root is
//! the per-entry set), a run-in flag read and executed, and `--allow-temp`
//! the private directory. A flag naming an excluded entry
//! (`--allow-write=.git`) is honored.
//!
//! Refused here, before anything is opened ([`PlanRefusal`]), each a
//! refusal the caller exits 2 for: `-A`; a write flag naming a directory
//! that holds the root, beneath which no rule can leave `.git` out; a
//! `node` that is a script, a version manager's shim, which can't read its
//! manager's directory inside the box, unless a run-in flag admits it; and
//! a `node_modules` symlink to anything but a directory named
//! `node_modules` outside the project, neither in it nor holding it, so a
//! committed link to `~/.ssh` or to `.git` gets no rule. Another
//! checkout's `node_modules` is followed, read and executed, and the plan
//! carries a note naming where it leads ([`BoxPlan::note`]); its tool
//! directories are made there only when a write flag names it, since that
//! checkout's own unconfined commands run what a test writes there.
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

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::rules::{Need, RealRights, Rights};

/// What a run is, as its box is derived from it: built from the verb's plan
/// and the run's one resolution of its pins and granted directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent facts of the run's closure and flags, each read on its own"
)]
pub struct BoxInputs {
    /// The project's root, a real path.
    pub root: PathBuf,
    /// Whether the run is unconfined (`-A`), which no box holds.
    pub unconfined: bool,
    /// Whether the command's closure writes the project's root, its check
    /// mode's writes dropped.
    pub writes_root: bool,
    /// The project's local programs the closure grants, each by its bare
    /// name (`vitest`, `svelte-kit`).
    pub tools: BTreeSet<String>,
    /// Whether the run may start a program: the closure grants one, or a
    /// flag does.
    pub spawns: bool,
    /// The packages the project's `package.json` declares in
    /// `dependencies` or `devDependencies`, by name.
    pub declared: BTreeSet<String>,
    /// The file each program the run pinned executes.
    pub programs: Vec<ProgramFile>,
    /// The directories the grant flags list, each resolved.
    pub flags: Vec<GrantedFlagDir>,
    /// Whether `--allow-temp` grants the realm's temporary directories,
    /// which the run makes in its private directory.
    pub temp: bool,
    /// `HOME`, if it's set.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, if it's set.
    pub xdg_config_home: Option<PathBuf>,
    /// `GIT_CONFIG_GLOBAL`, if it's set.
    pub git_config_global: Option<PathBuf>,
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
    /// Git's home config that exists, each by its real path: the files
    /// and the directory git reads from the home directory.
    pub git_home: Vec<PathBuf>,
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
    },
}

/// The run's box, planned: each rule as a path, its rights, and its
/// source, and what the plan left out and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxPlan {
    /// The project's root, a real path.
    root: PathBuf,
    /// The rules, in the order they open.
    rules: Vec<PlannedRule>,
    /// Whether the run gets a directory private to it.
    private_temp: bool,
    /// What got no rule, and why.
    skipped: Vec<Skipped>,
    /// Where the project's `node_modules` leads, when it's a symlink.
    node_modules_link: Option<PathBuf>,
    /// Whether a write flag names where `node_modules` leads, so its tool
    /// directories are made there.
    node_modules_written: bool,
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
    /// The run's private directory.
    PrivateTemp,
    /// A grant flag.
    Flag(DirFlag),
}

/// A directory a tool writes, made before the box when the command's
/// closure grants a local tool and the project declares what writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolDir {
    /// `.svelte-kit`, for `@sveltejs/kit`.
    SvelteKit,
    /// `node_modules/$app`, for `@sveltejs/kit`.
    App,
    /// `node_modules/.vite`, for `vite` or `vitest`.
    Vite,
    /// `node_modules/.vite-temp`, for `vite` or `vitest`.
    ViteTemp,
    /// `.fuz/cache`, for `@fuzdev/fuz_css` under `vitest`, whose plugin
    /// writes it as vitest transforms.
    FuzCache,
}

impl ToolDir {
    /// Its path beneath the project's root, or beneath `node_modules` for
    /// those [`ToolDir::in_node_modules`] names.
    pub const fn name(self) -> &'static str {
        match self {
            Self::SvelteKit => ".svelte-kit",
            Self::App => "$app",
            Self::Vite => ".vite",
            Self::ViteTemp => ".vite-temp",
            Self::FuzCache => ".fuz/cache",
        }
    }

    /// Whether it sits in `node_modules`.
    pub const fn in_node_modules(self) -> bool {
        matches!(self, Self::App | Self::Vite | Self::ViteTemp)
    }
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
    /// A write flag names a directory that holds the project's root.
    #[error(
        "{}={dir} names {}, which holds the project, and no kernel rule beneath it can leave the \
         project's .git and node_modules out: name directories inside the project or beside it",
        DirFlag::Write,
        real.display()
    )]
    WriteAboveRoot {
        /// The directory as written.
        dir: String,
        /// Its real path.
        real: PathBuf,
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

/// The devices every boxed run reads.
const READ_DEVICES: [&str; 3] = ["/dev/zero", "/dev/urandom", "/dev/random"];

/// The package that declares `.svelte-kit` and `node_modules/$app`.
const SVELTEKIT_PACKAGE: &str = "@sveltejs/kit";

/// The packages that declare `node_modules/.vite` and `.vite-temp`.
const VITE_PACKAGES: [&str; 2] = ["vite", "vitest"];

/// The package whose plugin writes `.fuz/cache` under `vitest`.
const FUZ_CSS_PACKAGE: &str = "@fuzdev/fuz_css";

/// The local program whose transforms run `@fuzdev/fuz_css`'s plugin.
const VITEST: &str = "vitest";

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
            skipped: Vec::new(),
            node_modules_link: match &facts.node_modules {
                NodeModules::Link { real } => Some(real.clone()),
                NodeModules::Absent | NodeModules::Directory => None,
            },
            node_modules_written: match &facts.node_modules {
                NodeModules::Link { real } => written_by_flag(inputs, real),
                NodeModules::Absent | NodeModules::Directory => false,
            },
        };
        let read_maybe_execute = if inputs.spawns {
            RealRights::ReadExecute
        } else {
            RealRights::Read
        };
        plan.beneath(Anchor::Project, ".", Rights::READ, RuleSource::Project);
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
        plan.system(facts);
        if inputs.writes_root || writes_root_by_flag(inputs) {
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

    /// Whether the run gets a directory private to it.
    pub const fn private_temp(&self) -> bool {
        self.private_temp
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
    /// that is a symlink, read and executed where it leads, and whether a
    /// write flag names it, which alone lets the tools make their
    /// directories there.
    pub fn note(&self) -> Option<String> {
        self.node_modules_link.as_ref().map(|real| {
            let tools = if self.node_modules_written {
                "and --allow-write= naming it lets the tools write their directories there"
            } else {
                "and no tool directory is made in it: --allow-write= naming it lets the tools \
                 write there"
            };
            format!(
                "note: node_modules is a symlink to {}, which the box follows to read and \
                 execute, {tools}",
                real.display()
            )
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
    /// `node_modules`, git's home config.
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
        for git in &facts.git_home {
            self.real(git, RealRights::Read, Need::Optional, RuleSource::GitHome);
        }
    }

    /// The tool directories the declared dependencies call for.
    fn add_tool_dirs(&mut self, inputs: &BoxInputs, facts: &BoxFacts) {
        let declares = |name: &str| inputs.declared.contains(name);
        let mut dirs = Vec::new();
        if declares(SVELTEKIT_PACKAGE) {
            dirs.extend([ToolDir::SvelteKit, ToolDir::App]);
        }
        if VITE_PACKAGES.iter().any(|name| declares(name)) {
            dirs.extend([ToolDir::Vite, ToolDir::ViteTemp]);
        }
        if inputs.tools.contains(VITEST) && declares(FUZ_CSS_PACKAGE) {
            dirs.push(ToolDir::FuzCache);
        }
        for dir in dirs {
            let (anchor, path) = if dir.in_node_modules() {
                match &facts.node_modules {
                    NodeModules::Directory => {
                        (Anchor::Project, Path::new("node_modules").join(dir.name()))
                    }
                    // another checkout's: written there only as a write flag
                    // names it, the command line's choice
                    NodeModules::Link { real } if written_by_flag(inputs, real) => {
                        (Anchor::Slash, beneath_root(real).join(dir.name()))
                    }
                    NodeModules::Absent | NodeModules::Link { .. } => continue,
                }
            } else {
                (Anchor::Project, PathBuf::from(dir.name()))
            };
            self.beneath(anchor, path, Rights::WRITE, RuleSource::ToolDir(dir));
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
                    self.beneath(
                        Anchor::Slash,
                        beneath_root(&flag.real),
                        Rights::READ | Rights::WRITE,
                        source,
                    );
                }
            }
        }
    }
}

/// Whether a write flag names `dir` or a directory holding it.
fn written_by_flag(inputs: &BoxInputs, dir: &Path) -> bool {
    inputs
        .flags
        .iter()
        .any(|flag| flag.flag == DirFlag::Write && dir.starts_with(&flag.real))
}

/// Whether a write flag names the project's root.
fn writes_root_by_flag(inputs: &BoxInputs) -> bool {
    inputs
        .flags
        .iter()
        .any(|flag| flag.flag == DirFlag::Write && flag.real == inputs.root)
}

/// The refusals the [module doc](self) lists, the first met.
fn refuse(inputs: &BoxInputs, facts: &BoxFacts) -> Result<(), PlanRefusal> {
    if inputs.unconfined {
        return Err(PlanRefusal::Unconfined);
    }
    if let Some(flag) = inputs.flags.iter().find(|flag| {
        flag.flag == DirFlag::Write
            && flag.real != inputs.root
            && inputs.root.starts_with(&flag.real)
    }) {
        return Err(PlanRefusal::WriteAboveRoot {
            dir: flag.dir.clone(),
            real: flag.real.clone(),
        });
    }
    if let NodeModules::Link { real } = &facts.node_modules
        && !(real.file_name() == Some(OsStr::new("node_modules"))
            && !real.starts_with(&inputs.root)
            && !inputs.root.starts_with(real))
    {
        return Err(PlanRefusal::NodeModulesLink { real: real.clone() });
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
