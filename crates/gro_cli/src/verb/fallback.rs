//! Why a command runs on the Node Gro instead of the binary: the closure's
//! verdicts, the binary's policies, and the project's installed Gro, each
//! with the reason the fallback prints and who decided it; and the run of
//! the Node Gro the project installed ([`exec_node_gro`]), which replaces
//! the process after one line saying why.
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
//! line the only sign; and one the host decides ([`DecidedBy::Host`]), a
//! kernel below the run's box's Landlock floor, is where the command runs,
//! which neither the user typing it nor a sandbox that launched it chose.
//! So each reason says who decided it, and a strict mode refuses every
//! fallback the command line didn't decide
//! ([`FallbackReason::is_refused_when_strict`]), `GRO_FALLBACK=command`
//! ([`FallbackPolicy`]).
//!
//! Kind of the strict mode: a boundary, a native check before anything
//! runs, read from the process's environment alone, never the project's
//! files. The adversary is a project that pins another Gro to leave the
//! binary, or a host that would run the project's code unconfined where
//! the box can't go up; the asset, what the binary confines. Unsetting the
//! variable defeats it, and so does running the project's own Gro some
//! other way (a package script's `gro`, `node_modules/.bin/gro`), each the
//! user's choice, not the project's.

use std::ffi::{OsStr, OsString};
use std::fmt::{self, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::Command;

use gro_runtime::confine::{ConfineError, REQUIRED_LANDLOCK_ABI};
use gro_runtime::find_on_path;
use gro_runtime::resolver::{GRO_PACKAGE, InstalledPackage};
use gro_runtime::verb::closure::{ClosureFallback, CommandClosure};
use thiserror::Error;

use super::prepass::{VerbFallback, command_line, joined_reasons};
use crate::error::CliError;
use crate::print::{escape_controls, stderr_line};

/// The environment variable naming which fallbacks to the Node Gro the
/// binary takes ([`FallbackPolicy`]).
pub const GRO_FALLBACK: &str = "GRO_FALLBACK";

/// Which fallbacks to the Node Gro the binary takes, as [`GRO_FALLBACK`]
/// names them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FallbackPolicy {
    /// `all`, and the variable unset: every fallback runs the Node Gro.
    #[default]
    All,
    /// `command`: only the fallbacks the command line decides; one the
    /// project's files or the host decide is refused, and nothing runs.
    Command,
}

impl FallbackPolicy {
    /// The policy `value`, [`GRO_FALLBACK`]'s value, names: unset is
    /// [`FallbackPolicy::All`].
    ///
    /// # Errors
    ///
    /// Returns [`FallbackPolicyError`] for any value but `all` and
    /// `command`, the empty one included, so a misspelled strict mode never
    /// runs as the default.
    pub fn read(value: Option<&OsStr>) -> Result<Self, FallbackPolicyError> {
        match value.map(|value| value.to_str()) {
            None | Some(Some("all")) => Ok(Self::All),
            Some(Some("command")) => Ok(Self::Command),
            Some(_) => Err(FallbackPolicyError(
                value.unwrap_or_default().to_string_lossy().into_owned(),
            )),
        }
    }

    /// Whether it refuses the fallback `reasons` make: under
    /// [`FallbackPolicy::Command`], when any is one the command line
    /// didn't decide.
    pub fn refuses(self, reasons: &[FallbackReason]) -> bool {
        self == Self::Command && reasons.iter().any(FallbackReason::is_refused_when_strict)
    }
}

/// [`GRO_FALLBACK`] set to a value it doesn't take, held as read, with
/// U+FFFD for each sequence that isn't UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "{GRO_FALLBACK} is \"{}\", and it takes all or command",
    escape_controls(.0)
)]
pub struct FallbackPolicyError(pub String);

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
    /// The host: a kernel that can't hold the run's box.
    Host,
}

/// What the kernel has of Landlock, where it's below the run's box's floor
/// (`gro_runtime::confine::run::probe_floor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KernelLandlock {
    /// No Landlock to reach: the kernel has none, or a seccomp filter on
    /// the process answers its calls `ENOSYS`, as a container's may.
    Absent,
    /// Built in, but not enabled: missing from the boot's `lsm=` list.
    Disabled,
    /// Landlock at this ABI, below the floor.
    Abi(u32),
}

impl KernelLandlock {
    /// What the floor probe's refusal `error` says the kernel has, or
    /// `None` for an error that isn't the floor's, as one reading the
    /// status.
    pub const fn from_floor_error(error: &ConfineError) -> Option<Self> {
        match error {
            ConfineError::NoLandlock { built: false } => Some(Self::Absent),
            ConfineError::NoLandlock { built: true } => Some(Self::Disabled),
            ConfineError::LandlockAbi { abi } => Some(Self::Abi(*abi)),
            _ => None,
        }
    }
}

impl fmt::Display for KernelLandlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let floor = format!("ABI {REQUIRED_LANDLOCK_ABI} (Linux 6.12)");
        match self {
            Self::Absent => write!(
                f,
                "Landlock isn't available here (the kernel has none, or a seccomp filter refuses \
                 its calls), which the kernel box needs at {floor} or later"
            ),
            Self::Disabled => write!(
                f,
                "the kernel's Landlock isn't enabled (it is missing from the boot's lsm= list), \
                 which the kernel box needs at {floor} or later"
            ),
            Self::Abi(abi) => write!(
                f,
                "the kernel's Landlock is ABI {abi}, below the {floor} the kernel box needs"
            ),
        }
    }
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
    /// The command is `task` with no task's path: it lists the project's
    /// tasks, importing each, whose grants no table holds.
    TaskListing,
    /// The kernel can't hold the run's box: its Landlock is below the
    /// floor the box needs.
    KernelFloor(KernelLandlock),
}

impl FallbackReason {
    /// Who decided it.
    pub const fn decided_by(&self) -> DecidedBy {
        match self {
            Self::VersionMismatch { .. } => DecidedBy::ProjectFiles,
            Self::Ungranted { .. }
            | Self::Policy(_)
            | Self::Install
            | Self::ProjectTask
            | Self::TaskListing => DecidedBy::Command,
            Self::KernelFloor(_) => DecidedBy::Host,
        }
    }

    /// Whether a strict mode refuses it rather than run the Node Gro: every
    /// fallback the command line didn't decide, the project's files' and
    /// the host's.
    pub const fn is_refused_when_strict(&self) -> bool {
        matches!(self.decided_by(), DecidedBy::ProjectFiles | DecidedBy::Host)
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
                    .filter(|installed| !installed.is_empty())
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
            Self::TaskListing => f.write_str(
                "`task` lists the project's tasks by importing each, and their grants are in no \
                 table",
            ),
            Self::KernelFloor(kernel) => kernel.fmt(f),
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

/// The program the Node Gro runs on, found on `PATH`.
const NODE: &str = "node";

/// Runs the command line `cli_args`, as written, grant flags included, on
/// the Node Gro the project installed, as `fallback` planned: prints one
/// line saying why ([`fallback_note`]), then replaces the process with
/// `node`, found on `path` as a pin finds a bare name ([`find_on_path`]:
/// absolute entries only, each `node_modules/.bin` one skipped), running
/// the installed Gro's `dist/invoke.js` with its loader registered as the
/// Node Gro registers it ([`loader_args`]), in the same working directory
/// and with the whole environment. Returns only when it can't.
///
/// Kind: the line is a review signal; what runs is the Node Gro, which
/// confines nothing.
pub fn exec_node_gro(
    fallback: &VerbFallback,
    cli_args: &[String],
    path: Option<&OsStr>,
    node_env: Option<&OsStr>,
) -> CliError {
    let command = fallback.command.clone();
    let dist = fallback.gro.root.join("dist");
    let invoke = dist.join("invoke.js");
    if !invoke.is_file() {
        return CliError::NodeGroUnbuilt {
            command,
            path: invoke,
        };
    }
    let path = path.map(OsStr::to_string_lossy);
    let node = match find_on_path(NODE, path.as_deref()) {
        Ok(Some(node)) => node,
        Ok(None) => return CliError::NoNode { command, why: None },
        Err(why) => {
            return CliError::NoNode {
                command,
                why: Some(why),
            };
        }
    };
    stderr_line(fallback_note(fallback));
    let args = loader_args(&dist.join("loader.js"), &invoke, cli_args, node_env);
    let source = Command::new(node).args(args).exec();
    CliError::NodeGroExec { command, source }
}

/// The line a fallback prints before it runs the Node Gro: `note: fgro
/// <command> runs on the Node gro in node_modules/@fuzdev/gro <version>,
/// which confines nothing, because <reason>[; and <reason>]`.
pub fn fallback_note(fallback: &VerbFallback) -> String {
    let version = fallback
        .gro
        .version
        .as_deref()
        .filter(|version| !version.is_empty())
        .map_or_else(String::new, |version| {
            format!(" {}", escape_controls(version))
        });
    format!(
        "note: {} runs on the Node gro in node_modules/{GRO_PACKAGE}{version}, which confines \
         nothing, because {}",
        command_line(fallback.command.as_deref()),
        joined_reasons(&fallback.reasons)
    )
}

/// The `NODE_ENV` value under which the Node Gro's loader doesn't match the
/// `development` condition.
const NODE_ENV_PRODUCTION: &[u8] = b"production";

/// The Node args that run `invoke_path` with `argv` after it, with the
/// loader at `loader_path` registered: the twin of Gro's
/// `spawn_with_loader_args` (`src/lib/gro_helpers.ts`), held to
/// `tests/fixtures/verb/loader_args.json`, which that function writes.
/// `-C development` is added unless `node_env` is `production`.
pub fn loader_args(
    loader_path: &Path,
    invoke_path: &Path,
    argv: &[String],
    node_env: Option<&OsStr>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--import".into(),
        register_loader_url(loader_path).into(),
        "--experimental-import-meta-resolve".into(),
    ];
    if node_env.is_none_or(|node_env| node_env.as_bytes() != NODE_ENV_PRODUCTION) {
        args.extend(["-C".into(), "development".into()]);
    }
    args.push(invoke_path.into());
    args.extend(argv.iter().map(OsString::from));
    args
}

/// The `data:` URL of a module that registers the loader at `loader_path`,
/// as Gro's `to_register_loader_url` makes it: the module's source,
/// `encodeURIComponent`'s encoding of it, after `data:text/javascript,`.
fn register_loader_url(loader_path: &Path) -> String {
    // the URL holds only what `to_file_url` keeps and `%`, which a JSON
    // string writes as they are
    let source = format!(
        "import {{register}} from \"node:module\"; register(\"{}\");",
        file_url(loader_path)
    );
    format!("data:text/javascript,{}", encode_uri_component(&source))
}

/// The `file:` URL of the absolute `path`, as Gro's `to_file_url` writes
/// it: the path's `.` and `..` resolved by their text, a trailing `/` kept,
/// each byte outside the characters a file URL's path keeps percent-encoded,
/// uppercase. A path that isn't UTF-8 has its bytes encoded as they are,
/// which a JS string can't hold.
fn file_url(path: &Path) -> String {
    let bytes = path.as_os_str().as_bytes();
    let mut segments: Vec<&[u8]> = Vec::new();
    for segment in bytes.split(|&byte| byte == b'/') {
        match segment {
            b"" | b"." => {}
            b".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    let mut resolved = Vec::with_capacity(bytes.len() + 1);
    for segment in &segments {
        resolved.push(b'/');
        resolved.extend_from_slice(segment);
    }
    // the root alone, or a trailing `/` kept
    if resolved.is_empty() || bytes.ends_with(b"/") {
        resolved.push(b'/');
    }
    let kept = |byte: u8| byte.is_ascii_alphanumeric() || b"!$&'()*+,-./:;=@_".contains(&byte);
    let mut url = String::from("file://");
    for byte in resolved {
        if kept(byte) {
            url.push(char::from(byte));
        } else {
            // writing to a `String` never fails
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

/// `text` as JS's `encodeURIComponent` encodes it: every UTF-8 byte outside
/// the letters, digits, and `-_.!~*'()` percent-encoded, uppercase.
fn encode_uri_component(text: &str) -> String {
    let kept = |byte: u8| byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte);
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if kept(byte) {
            encoded.push(char::from(byte));
        } else {
            // writing to a `String` never fails
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gro_runtime::resolver::InstalledPackage;
    use gro_runtime::verb::closure::command_closure;
    use gro_runtime::verb::table::BuiltinGrantsTable;

    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use serde_json::Value;

    use super::{
        BUILTIN_ROWS, BuiltinRow, DecidedBy, FallbackPolicy, FallbackReason, Policy,
        fallback_reasons, loader_args,
    };
    use crate::verb::tests::{has_fields, read_fixture, strings};

    /// The Node args are `spawn_with_loader_args`'s, case by case.
    #[test]
    fn the_loader_args_are_node_s() {
        let fixture = read_fixture("loader_args.json");
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() > 1);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            assert!(
                has_fields(
                    case,
                    &[
                        "name",
                        "loader_path",
                        "invoke_path",
                        "argv",
                        "node_env",
                        "args"
                    ]
                ),
                "{name}"
            );
            let node_env = case["node_env"].as_str().map(OsStr::new);
            let args = loader_args(
                Path::new(case["loader_path"].as_str().unwrap()),
                Path::new(case["invoke_path"].as_str().unwrap()),
                &strings(&case["argv"]),
                node_env,
            );
            let expected: Vec<OsString> = strings(&case["args"])
                .into_iter()
                .map(OsString::from)
                .collect();
            assert_eq!(args, expected, "{name}");
            assert!(matches!(case["node_env"], Value::String(_) | Value::Null));
        }
    }

    /// `GRO_FALLBACK` takes `all` and `command`, and unset is `all`.
    #[test]
    fn the_fallback_policy_is_read_from_its_value() {
        assert_eq!(FallbackPolicy::read(None), Ok(FallbackPolicy::All));
        assert_eq!(
            FallbackPolicy::read(Some(OsStr::new("all"))),
            Ok(FallbackPolicy::All)
        );
        assert_eq!(
            FallbackPolicy::read(Some(OsStr::new("command"))),
            Ok(FallbackPolicy::Command)
        );
        let invalid = {
            use std::os::unix::ffi::OsStrExt as _;
            OsStr::from_bytes(b"comm\xffand")
        };
        assert_eq!(
            FallbackPolicy::read(Some(invalid)).unwrap_err().to_string(),
            "GRO_FALLBACK is \"comm\u{fffd}and\", and it takes all or command"
        );
    }

    /// The note names the installed version escaped, as the project wrote
    /// it, and leaves out an empty one.
    #[test]
    fn the_note_escapes_the_installed_version() {
        use gro_runtime::resolver::InstalledPackage;

        use super::fallback_note;
        use crate::verb::prepass::VerbFallback;

        let note = |version: &str| {
            fallback_note(&VerbFallback {
                command: Some("dev".to_owned()),
                reasons: vec![FallbackReason::Install],
                gro: InstalledPackage {
                    root: PathBuf::from("/p/node_modules/@fuzdev/gro"),
                    version: Some(version.to_owned()),
                },
            })
        };
        let hostile = note("1.0.0\u{1b}[2J\r");
        assert!(
            hostile.contains("@fuzdev/gro 1.0.0\\u{1b}[2J\\u{d}, which confines nothing"),
            "{hostile}"
        );
        let empty = note("");
        assert!(
            empty.contains("node_modules/@fuzdev/gro, which confines nothing"),
            "{empty}"
        );
    }

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
        // an empty version is none
        assert_eq!(mismatch(Some("")), mismatch(None));
        assert_eq!(
            reasons(&["dev"], None)[0].to_string(),
            "`dev` declares no grants: it serves and watches the project with Vite and its plugins"
        );
    }
}
