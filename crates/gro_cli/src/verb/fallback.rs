//! Why a command runs on the Node Gro instead of the binary: the closure's
//! verdicts, the binary's policies, and the project's installed Gro, each
//! with the reason the fallback prints and who decided it.
//!
//! [`BUILTIN_ROWS`] holds one row per builtin of the grants table, and its
//! test holds the two to each other: a builtin that declares no grants has
//! its reason here, and one the binary leaves to Node by policy, though its
//! grants say what it reaches, says why.
//!
//! Kind: the fallback line is a review signal. A fallback the command line
//! decides ([`DecidedBy::Command`]) is the user's choice; one the project's
//! files decide ([`DecidedBy::ProjectFiles`]) is the project author's, the
//! adversary for code the user hasn't vetted, who can pin another Gro in
//! one line of `package.json` and so run the project's code unconfined, the
//! line the only sign. So each reason says who decided it, and a strict
//! mode refuses every fallback the project's files decide
//! ([`FallbackReason::is_refused_when_strict`]).

use std::fmt;

use gro_runtime::resolver::{GRO_PACKAGE, InstalledPackage};
use gro_runtime::verb::closure::{ClosureFallback, CommandClosure};

use crate::print::escape_controls;

/// What decides that a builtin runs where it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinRow {
    /// Its closure: it runs on the binary unless the closure says otherwise.
    Closure,
    /// It declares no grants, so it runs on the Node Gro, for this reason.
    Ungranted(&'static str),
    /// The binary's policy runs it on the Node Gro, whatever its grants.
    Policy(Policy),
}

/// A builtin the binary runs on the Node Gro by policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Policy {
    /// `clean` removes top-level directories, which needs write access to the
    /// project's root, where `.git` is.
    Clean,
}

/// Each builtin of the grants table by name, and what decides where it
/// runs.
pub const BUILTIN_ROWS: &[(&str, BuiltinRow)] = &[
    (
        "build",
        BuiltinRow::Ungranted("it runs Vite's build with the project's plugins and adapter"),
    ),
    (
        "changeset",
        BuiltinRow::Ungranted("it runs Changesets, which reads git and writes release notes"),
    ),
    ("check", BuiltinRow::Closure),
    ("clean", BuiltinRow::Policy(Policy::Clean)),
    (
        "commit",
        BuiltinRow::Ungranted("it commits and pushes with git, reaching the network"),
    ),
    (
        "deploy",
        BuiltinRow::Ungranted("it builds and pushes a branch, reaching the network"),
    ),
    (
        "dev",
        BuiltinRow::Ungranted("it serves and watches the project with Vite and its plugins"),
    ),
    ("format", BuiltinRow::Closure),
    ("gen", BuiltinRow::Closure),
    ("lint", BuiltinRow::Closure),
    (
        "publish",
        BuiltinRow::Ungranted(
            "it publishes to the registry and pushes with git, reaching the network",
        ),
    ),
    (
        "reinstall",
        BuiltinRow::Ungranted("it reinstalls packages with npm, reaching the network"),
    ),
    ("release", BuiltinRow::Closure),
    (
        "run",
        BuiltinRow::Ungranted("it runs a script with Node's loader, the script's reach its own"),
    ),
    ("sync", BuiltinRow::Closure),
    ("test", BuiltinRow::Closure),
    ("typecheck", BuiltinRow::Closure),
    (
        "upgrade",
        BuiltinRow::Ungranted("it upgrades packages with npm, reaching the network"),
    ),
];

/// Who decided a fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// The command line: the command, its args, or a builtin it runs.
    Command,
    /// The project's files, which the project's author wrote.
    ProjectFiles,
}

/// Why a command runs on the Node Gro.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FallbackReason {
    /// The project's installed Gro is another version than the binary's,
    /// so the project's own Gro runs, as it pins.
    VersionMismatch {
        /// The installed Gro's version, as its `package.json` gives it, or
        /// none.
        installed: Option<String>,
        /// The binary's.
        binary: String,
    },
    /// A builtin the command runs declares no grants.
    Ungranted {
        /// The builtin.
        builtin: String,
        /// Why it declares none, its row's, which every builtin the table
        /// holds has.
        why: Option<&'static str>,
    },
    /// A builtin the command runs is the binary's policy to leave to Node.
    Policy(Policy),
    /// The command installs packages with npm, which reaches the network.
    Install,
    /// The command runs a project task, whose grants no table holds.
    ProjectTask,
}

impl FallbackReason {
    /// Who decided it.
    pub const fn decided_by(&self) -> DecidedBy {
        match self {
            Self::VersionMismatch { .. } => DecidedBy::ProjectFiles,
            Self::Ungranted { .. } | Self::Policy(_) | Self::Install | Self::ProjectTask => {
                DecidedBy::Command
            }
        }
    }

    /// Whether a strict mode refuses it rather than run the Node Gro: every
    /// fallback the project's files decide.
    pub const fn is_refused_when_strict(&self) -> bool {
        matches!(self.decided_by(), DecidedBy::ProjectFiles)
    }
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionMismatch { installed, binary } => write!(
                f,
                "the project installed {GRO_PACKAGE} {}, and this is {binary}, so the project's \
                 own runs",
                installed
                    .as_deref()
                    .map_or_else(|| "with no version".into(), escape_controls)
            ),
            Self::Ungranted { builtin, why } => {
                write!(f, "`{builtin}` declares no grants")?;
                why.map_or(Ok(()), |why| write!(f, ": {why}"))
            }
            Self::Policy(Policy::Clean) => f.write_str(
                "`clean` removes top-level directories, which needs write access to the \
                 project's root, where .git is",
            ),
            Self::Install => f.write_str("installing packages runs npm, which reaches the network"),
            Self::ProjectTask => {
                f.write_str("a project task's grants are in no table, so the Node Gro runs it")
            }
        }
    }
}

/// Why the command whose closure is `closure` runs on the Node Gro, none
/// when it runs on the binary: the project's installed Gro, `installed_gro`,
/// when it's another version than `binary_version`, the binary's, then the
/// closure's verdicts and the policies of the builtins it runs, each once,
/// those the project's files decide first.
pub fn fallback_reasons(
    closure: &CommandClosure,
    installed_gro: Option<&InstalledPackage>,
    binary_version: &str,
) -> Vec<FallbackReason> {
    let mut reasons = Vec::new();
    if let Some(installed) = installed_gro
        && installed.version.as_deref() != Some(binary_version)
    {
        reasons.push(FallbackReason::VersionMismatch {
            installed: installed.version.clone(),
            binary: binary_version.to_owned(),
        });
    }
    for fallback in &closure.fallback {
        reasons.push(match fallback {
            ClosureFallback::Ungranted { builtin } => FallbackReason::Ungranted {
                builtin: builtin.clone(),
                why: match builtin_row(builtin) {
                    Some(BuiltinRow::Ungranted(why)) => Some(why),
                    _ => None,
                },
            },
            ClosureFallback::Install => FallbackReason::Install,
            ClosureFallback::ProjectTask => FallbackReason::ProjectTask,
        });
    }
    for builtin in &closure.builtins {
        if let Some(BuiltinRow::Policy(policy)) = builtin_row(&builtin.name) {
            reasons.push(FallbackReason::Policy(policy));
        }
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

/// The row of the builtin `name`.
fn builtin_row(name: &str) -> Option<BuiltinRow> {
    BUILTIN_ROWS
        .iter()
        .find_map(|&(row_name, row)| (row_name == name).then_some(row))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gro_runtime::resolver::InstalledPackage;
    use gro_runtime::verb::closure::command_closure;
    use gro_runtime::verb::table::BuiltinGrantsTable;

    use super::{BUILTIN_ROWS, BuiltinRow, DecidedBy, FallbackReason, Policy, fallback_reasons};

    #[test]
    fn every_builtin_has_its_row() {
        let table = BuiltinGrantsTable::carried().unwrap();
        let names: Vec<&str> = BUILTIN_ROWS.iter().map(|&(name, _)| name).collect();
        assert!(names.is_sorted(), "the rows are sorted by name: {names:?}");
        assert_eq!(
            names,
            table
                .builtins
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        for &(name, row) in BUILTIN_ROWS {
            let declares_grants = table.builtins[name].is_some();
            match row {
                // a reason exactly where the table holds none
                BuiltinRow::Ungranted(why) => {
                    assert!(!declares_grants && !why.is_empty(), "{name}");
                }
                BuiltinRow::Closure | BuiltinRow::Policy(_) => assert!(declares_grants, "{name}"),
            }
        }
    }

    /// Gro installed in the project at `version`, or with none.
    fn gro(version: Option<&str>) -> InstalledPackage {
        InstalledPackage {
            root: PathBuf::from("/p/node_modules/@fuzdev/gro"),
            version: version.map(str::to_owned),
        }
    }

    fn reasons(cli_args: &[&str], installed: Option<&InstalledPackage>) -> Vec<FallbackReason> {
        let table = BuiltinGrantsTable::carried().unwrap();
        let cli_args: Vec<String> = cli_args.iter().map(|&arg| arg.to_owned()).collect();
        fallback_reasons(&command_closure(&table, &cli_args), installed, "1.2.3")
    }

    #[test]
    fn a_command_runs_on_node_for_its_closure_s_verdicts_and_the_policies() {
        assert_eq!(reasons(&["check"], None), []);
        assert_eq!(reasons(&["check"], Some(&gro(Some("1.2.3")))), []);
        assert_eq!(
            reasons(&["clean"], None),
            [FallbackReason::Policy(Policy::Clean)]
        );
        assert_eq!(
            reasons(&["sync", "--install"], None),
            [FallbackReason::Install]
        );
        assert_eq!(
            reasons(&["check", "--install", "--build"], None),
            [
                FallbackReason::Ungranted {
                    builtin: "build".to_owned(),
                    why: Some("it runs Vite's build with the project's plugins and adapter"),
                },
                FallbackReason::Install,
            ]
        );
        assert_eq!(
            reasons(&["task", "src/a.task.ts"], None),
            [FallbackReason::ProjectTask]
        );
        // every reason is decided by the command line, and no strict mode refuses one
        for reason in reasons(&["release", "--install"], None)
            .into_iter()
            .chain(reasons(&["clean"], None))
            .chain(reasons(&["task", "a.task.ts"], None))
        {
            assert_eq!(reason.decided_by(), DecidedBy::Command, "{reason}");
            assert!(!reason.is_refused_when_strict());
        }
    }

    #[test]
    fn another_installed_gro_runs_decided_by_the_project_s_files() {
        let mismatch = |installed: Option<&str>| FallbackReason::VersionMismatch {
            installed: installed.map(str::to_owned),
            binary: "1.2.3".to_owned(),
        };
        for installed in [Some("1.2.2"), Some("1.2.3-next.1"), Some("v1.2.3"), None] {
            let reasons = reasons(&["check"], Some(&gro(installed)));
            assert_eq!(reasons, [mismatch(installed)]);
            assert_eq!(reasons[0].decided_by(), DecidedBy::ProjectFiles);
            assert!(reasons[0].is_refused_when_strict());
        }
        // first, beside the command's own
        assert_eq!(
            reasons(&["clean"], Some(&gro(Some("0.1.0")))),
            [
                mismatch(Some("0.1.0")),
                FallbackReason::Policy(Policy::Clean)
            ]
        );
    }

    #[test]
    fn a_reason_prints_what_decided_it() {
        let mismatch = |installed: Option<&str>| {
            FallbackReason::VersionMismatch {
                installed: installed.map(str::to_owned),
                binary: "1.2.3".to_owned(),
            }
            .to_string()
        };
        assert_eq!(
            mismatch(Some("0.9.0\u{1b}[2J")),
            "the project installed @fuzdev/gro 0.9.0\\u{1b}[2J, and this is 1.2.3, so the \
             project's own runs"
        );
        assert_eq!(
            mismatch(None),
            "the project installed @fuzdev/gro with no version, and this is 1.2.3, so the \
             project's own runs"
        );
        assert_eq!(
            reasons(&["dev"], None)[0].to_string(),
            "`dev` declares no grants: it serves and watches the project with Vite and its plugins"
        );
    }
}
