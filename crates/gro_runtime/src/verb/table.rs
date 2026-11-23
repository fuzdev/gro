//! The grants table: what the CLI's own steps and each of Gro's builtin
//! tasks may reach, `src/builtin_grants.json`, carried in the binary and
//! read before any JS runs.
//!
//! The TS table's genfile (`src/lib/gro_builtin_tasks.gen.ts`) writes the
//! JSON beside `GRO_CLI_GRANTS` and `GRO_BUILTIN_TASK_GRANTS`, and
//! `src/test/builtin_grants.json.test.ts` holds the two renders equal, so
//! the types here are the TS ones (`BuiltinTaskGrants` and its parts in
//! `src/lib/builtin_host.ts`), each field closed: an unknown field, a mark
//! other than `true`, or a shape the TS types don't allow is refused. Beside
//! the grants, the table holds each builtin's args as its `Args` declares
//! them ([`BuiltinArgs`]), which the closure reads a command line against.
//! Beside the shape, a table is refused where the closure couldn't be read
//! from it ([`TableError`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// `src/builtin_grants.json`, as the binary carries it.
pub const BUILTIN_GRANTS_JSON: &str = include_str!("../../../../src/builtin_grants.json");

/// The grants table: the CLI's own row and each builtin's, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinGrantsTable {
    /// The grants of the CLI's own steps before any task: resolving a task
    /// path, loading the config, and listing the project's tasks.
    pub cli: BuiltinTaskGrants,
    /// Each builtin's grants by name, `None` for one that declares none,
    /// whose reach nothing states.
    pub builtins: BTreeMap<String, Option<BuiltinTaskGrants>>,
    /// Each builtin's args by name, `None` for one with no `Args`, whose
    /// args Node passes unparsed.
    pub args: BTreeMap<String, Option<BuiltinArgs>>,
}

/// The args a builtin's `Args` holds, by name: a boolean's default, `None`
/// for any other arg.
///
/// Node's `args_parse` refuses a key it doesn't hold and a value given a
/// boolean, and syncs a boolean `no-x` with a boolean `x`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct BuiltinArgs(pub BTreeMap<String, Option<bool>>);

impl BuiltinArgs {
    /// Whether `key` is an arg it holds.
    pub fn holds(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// The default of the boolean arg `key`, `None` for an arg that isn't a
    /// boolean or isn't held.
    pub fn boolean_default(&self, key: &str) -> Option<bool> {
        self.0.get(key).copied().flatten()
    }
}

impl BuiltinGrantsTable {
    /// The table the binary carries, [`BUILTIN_GRANTS_JSON`].
    ///
    /// # Errors
    ///
    /// Returns why it isn't a table, which `src/builtin_grants.json` as
    /// committed never is: its test reads it.
    pub fn carried() -> Result<Self, TableError> {
        Self::parse(BUILTIN_GRANTS_JSON)
    }

    /// Reads the table `json` holds.
    ///
    /// # Errors
    ///
    /// Returns [`TableError::Json`] for JSON that isn't a table's shape,
    /// and the other variants for a table the closure can't be read from.
    pub fn parse(json: &str) -> Result<Self, TableError> {
        let TableJson {
            _description,
            cli,
            builtins,
            args,
        } = serde_json::from_str(json).map_err(TableError::Json)?;
        if let Some(name) = builtins
            .keys()
            .find(|name| !args.contains_key(*name))
            .or_else(|| args.keys().find(|name| !builtins.contains_key(*name)))
        {
            return Err(TableError::ArgsUnmatched(name.clone()));
        }
        if let Some(invoked) = cli.invokes.keys().next() {
            return Err(TableError::CliInvokes(invoked.clone()));
        }
        for (name, grants) in &builtins {
            for (invoked, invoke) in grants.iter().flat_map(|grants| &grants.invokes) {
                if let Some(when) = &invoke.when
                    && args
                        .get(name)
                        .and_then(Option::as_ref)
                        .and_then(|args| args.boolean_default(when))
                        .is_none()
                {
                    return Err(TableError::WhenNotBoolean {
                        builtin: name.clone(),
                        invoked: invoked.clone(),
                        when: when.clone(),
                    });
                }
                if !builtins.contains_key(invoked) {
                    return Err(TableError::UnknownInvoke {
                        builtin: name.clone(),
                        invoked: invoked.clone(),
                    });
                }
                let invoked_grants = builtins.get(invoked).and_then(Option::as_ref);
                if let Some((when_invoked, when)) = invoked_grants
                    .into_iter()
                    .flat_map(|grants| &grants.invokes)
                    .find_map(|(when_invoked, invoke)| {
                        invoke.when.as_ref().map(|when| (when_invoked, when))
                    })
                {
                    return Err(TableError::WhenBeyondCommand {
                        builtin: invoked.clone(),
                        invoked: when_invoked.clone(),
                        when: when.clone(),
                        invoker: name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            cli,
            builtins,
            args,
        })
    }
}

impl BuiltinGrantsTable {
    /// The args of the builtin `name`, `None` for one with no `Args` or a
    /// name the table doesn't hold.
    pub fn args_of(&self, name: &str) -> Option<&BuiltinArgs> {
        self.args.get(name).and_then(Option::as_ref)
    }

    /// Whether the builtin `name` has a check mode the table knows of:
    /// some invoke of it is marked `check`.
    pub fn is_check_capable(&self, name: &str) -> bool {
        self.builtins
            .values()
            .flatten()
            .any(|grants| grants.invokes.get(name).is_some_and(|invoke| invoke.check))
    }
}

/// Why a table can't be read.
#[derive(Debug, Error)]
pub enum TableError {
    /// It isn't the table's shape: not JSON, a field missing or unknown, or
    /// a value of another type, a mark other than `true` among them.
    #[error("the grants table isn't the TS table's shape")]
    Json(#[source] serde_json::Error),
    /// The CLI's own row invokes a builtin, which no step before a task
    /// does.
    #[error("the grants table's CLI row invokes `{0}`, and the CLI's own steps invoke no builtin")]
    CliInvokes(String),
    /// A builtin's grants and its args aren't listed for the same names.
    #[error("the grants table lists `{0}` in one of its grants and its args, not both")]
    ArgsUnmatched(String),
    /// An invoke marked `when` names no boolean arg with a default of its
    /// invoker's.
    #[error(
        "the grants table's `{builtin}` invokes `{invoked}` when its `{when}` arg is on, and \
         `{builtin}`'s args hold no boolean `{when}` with a default"
    )]
    WhenNotBoolean {
        /// The invoking builtin.
        builtin: String,
        /// The builtin it invokes.
        invoked: String,
        /// The arg.
        when: String,
    },
    /// A builtin invokes a name the table doesn't hold.
    #[error("the grants table's `{builtin}` invokes `{invoked}`, which isn't a builtin")]
    UnknownInvoke {
        /// The invoking builtin.
        builtin: String,
        /// The name it invokes.
        invoked: String,
    },
    /// A builtin another invokes has an invoke marked `when`: whether it
    /// runs is the invoked builtin's args, which its invoker's code passes,
    /// so no command line says, and the closure reads only the command's.
    #[error(
        "the grants table's `{builtin}` invokes `{invoked}` when its `{when}` arg is set, and \
         `{invoker}` invokes `{builtin}`, whose args the closure can't read"
    )]
    WhenBeyondCommand {
        /// The builtin with the marked invoke.
        builtin: String,
        /// The builtin it invokes when the arg is set.
        invoked: String,
        /// The arg.
        when: String,
        /// A builtin that invokes it.
        invoker: String,
    },
}

/// `src/builtin_grants.json` as the binary reads it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableJson {
    #[serde(rename = "description")]
    _description: String,
    cli: BuiltinTaskGrants,
    builtins: BTreeMap<String, Option<BuiltinTaskGrants>>,
    args: BTreeMap<String, Option<BuiltinArgs>>,
}

/// What a builtin task, or the CLI's own steps, may reach: the TS
/// `BuiltinTaskGrants`, a field left out in the JSON read as empty or
/// unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinTaskGrants {
    /// The directories it reads and writes.
    #[serde(default)]
    pub fs: BuiltinFsGrant,
    /// The environment variables it reads, exact names.
    #[serde(default)]
    pub env: Vec<String>,
    /// The programs it starts, by role.
    #[serde(default)]
    pub programs: BTreeMap<String, BuiltinProgramGrant>,
    /// Whether it loads the project's modules, as code of the project's.
    #[serde(default, deserialize_with = "literal_true")]
    pub project_modules: bool,
    /// Whether it writes to standard output.
    #[serde(default, deserialize_with = "literal_true")]
    pub stdout: bool,
    /// Whether it writes to standard error.
    #[serde(default, deserialize_with = "literal_true")]
    pub stderr: bool,
    /// The builtins it invokes, by name, each with what narrows the invoke.
    #[serde(default)]
    pub invokes: BTreeMap<String, BuiltinInvokeGrant>,
}

/// The directories a builtin reaches, each relative to the project root,
/// `.` the project: the TS `BuiltinFsGrant`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinFsGrant {
    /// The directories it reads.
    #[serde(default)]
    pub read: Vec<String>,
    /// The directories it writes.
    #[serde(default)]
    pub write: Vec<String>,
}

/// A program a builtin starts, keyed by its role: the TS
/// `BuiltinProgramGrant`, each field kept as declared, present or not, so
/// two builtins granting a role alike are one grant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinProgramGrant {
    /// The program it runs, the only one its role runs.
    pub name: String,
    /// Whether it's the project's own install, `node_modules/.bin/<name>`,
    /// never one on `PATH`.
    #[serde(default, deserialize_with = "literal_true")]
    pub local: bool,
    /// The host variables it sees beyond the base set.
    #[serde(default, deserialize_with = "not_null")]
    pub pass_env: Option<PassEnv>,
    /// The variables left out of what `pass_env` passes.
    #[serde(default, deserialize_with = "not_null")]
    pub omit_env: Option<Vec<String>>,
    /// The variables set for it, whatever the host's are.
    #[serde(default, deserialize_with = "not_null")]
    pub env: Option<BTreeMap<String, String>>,
}

/// The host variables a program sees beyond the base set: every one, or
/// those named, exact names or `PREFIX_*` globs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(untagged)]
pub enum PassEnv {
    /// `"all"`.
    All(AllKeyword),
    /// The names, as listed.
    Names(Vec<String>),
}

/// The keyword `"all"`, the one string `pass_env` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub enum AllKeyword {
    /// `"all"`.
    #[serde(rename = "all")]
    All,
}

/// How a builtin invokes another: the TS `BuiltinInvokeGrant`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinInvokeGrant {
    /// A boolean arg of the invoking builtin, the invoke running only when
    /// it's `true` once parsed, its default and its `no-` dual applied.
    #[serde(default, deserialize_with = "not_null")]
    pub when: Option<String>,
    /// Whether the invoke runs in check mode, writing no source, so the
    /// closure holds the invoked builtin, and those it invokes in turn,
    /// without their writes to the project.
    #[serde(default, deserialize_with = "literal_true")]
    pub check: bool,
}

/// Reads a field the TS types as optional, present or absent: a value is
/// set, and `null` is refused, since no TS table holds it and reading it as
/// absent would widen what the field narrows (an invoke with no `when`, a
/// program with no `omit_env`).
fn not_null<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Reads a mark the TS types as `true`, present or absent: `true` is set,
/// and `false` is refused, since no TS table holds it.
fn literal_true<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    if bool::deserialize(deserializer)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom(
            "a mark is `true` or left out, never `false`",
        ))
    }
}

/// The JSON of a test table's `args` for `builtins`, the JSON of each
/// builtin's row by name: each holds the booleans `x`, `build`, `install`,
/// and `check`, `false` by default, and `on`, `true` by default, with its
/// dual `no-on`.
#[cfg(test)]
pub(crate) fn test_args(builtins: &str) -> String {
    let builtins: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(builtins).unwrap_or_default();
    let args: serde_json::Map<String, serde_json::Value> = builtins
        .keys()
        .map(|name| {
            (
                name.clone(),
                serde_json::json!({
                    "x": false, "build": false, "install": false, "check": false,
                    "on": true, "no-on": false
                }),
            )
        })
        .collect();
    serde_json::Value::Object(args).to_string()
}

#[cfg(test)]
mod tests {
    use super::{BuiltinGrantsTable, PassEnv, TableError, test_args};

    /// A table of `builtins`, the JSON of each builtin's row by name, with an
    /// empty CLI row and [`test_args`].
    fn table(builtins: &str) -> Result<BuiltinGrantsTable, TableError> {
        table_with_args(builtins, &test_args(builtins))
    }

    fn table_with_args(builtins: &str, args: &str) -> Result<BuiltinGrantsTable, TableError> {
        BuiltinGrantsTable::parse(&format!(
            r#"{{"description": "", "cli": {{}}, "builtins": {builtins}, "args": {args}}}"#
        ))
    }

    #[test]
    fn the_carried_table_is_the_ts_table() {
        let table = BuiltinGrantsTable::carried().unwrap();
        assert!(table.cli.fs.read == ["."] && table.cli.project_modules);
        assert!(table.cli.invokes.is_empty() && table.cli.programs.is_empty());
        // a builtin that declares none, and one that declares each kind of field
        assert_eq!(table.builtins.get("dev"), Some(&None));
        let check = table.builtins["check"].as_ref().unwrap();
        assert_eq!(check.invokes["build"].when.as_deref(), Some("build"));
        assert_eq!(check.invokes["test"].when.as_deref(), Some("test"));
        let check_args = table.args_of("check").unwrap();
        assert_eq!(check_args.boolean_default("build"), Some(false));
        assert_eq!(check_args.boolean_default("test"), Some(true));
        assert_eq!(check_args.boolean_default("no-test"), Some(false));
        assert!(table.args_of("gen").unwrap().holds("_"));
        assert_eq!(table.args_of("gen").unwrap().boolean_default("_"), None);
        assert!(check.invokes["gen"].check && !check.invokes["test"].check);
        let typecheck = table.builtins["typecheck"].as_ref().unwrap();
        let svelte_check = &typecheck.programs["svelte_check_cli"];
        assert!(svelte_check.local && svelte_check.name == "svelte-check");
        assert!(matches!(svelte_check.pass_env, Some(PassEnv::All(_))));
        assert_eq!(svelte_check.env.as_ref().unwrap()["FORCE_COLOR"], "1");
        let sync = table.builtins["sync"].as_ref().unwrap();
        assert!(sync.programs["npm_cli"].omit_env.as_ref().unwrap().len() > 1);
        assert!(sync.stderr && !sync.stdout && sync.fs.write == ["."]);
    }

    #[test]
    fn a_table_s_shape_is_the_ts_types() {
        assert!(table(r#"{"a": {}}"#).is_ok());
        for refused in [
            // an unknown field, at every level
            r#"{"a": {"net": true}}"#,
            r#"{"a": {"fs": {"exec": ["."]}}}"#,
            r#"{"a": {"programs": {"x_cli": {"name": "x", "path": "/x"}}}}"#,
            r#"{"a": {"invokes": {"a": {"unless": "x"}}}}"#,
            // a mark is `true` or absent
            r#"{"a": {"stdout": false}}"#,
            r#"{"a": {"invokes": {"a": {"check": false}}}}"#,
            r#"{"a": {"programs": {"x_cli": {"name": "x", "local": false}}}}"#,
            // `pass_env` is `"all"` or names
            r#"{"a": {"programs": {"x_cli": {"name": "x", "pass_env": "some"}}}}"#,
            // a program names its program
            r#"{"a": {"programs": {"x_cli": {}}}}"#,
            // an optional field is present or absent, never `null`
            r#"{"a": {"invokes": {"a": {"when": null}}}}"#,
            r#"{"a": {"programs": {"x_cli": {"name": "x", "pass_env": null}}}}"#,
            r#"{"a": {"programs": {"x_cli": {"name": "x", "omit_env": null}}}}"#,
            r#"{"a": {"programs": {"x_cli": {"name": "x", "env": null}}}}"#,
        ] {
            assert!(
                matches!(table(refused), Err(TableError::Json(_))),
                "{refused}"
            );
        }
        assert!(matches!(
            BuiltinGrantsTable::parse(r#"{"cli": {}, "builtins": {}, "args": {}}"#),
            Err(TableError::Json(_))
        ));
        // the args are a boolean's default or `null`, or `null` for a builtin with no `Args`
        assert!(table_with_args(r#"{"a": {}}"#, r#"{"a": null}"#).is_ok());
        assert!(table_with_args(r#"{"a": {}}"#, r#"{"a": {"b": null, "c": true}}"#).is_ok());
        for refused in [r#"{"a": {"b": 1}}"#, r#"{"a": {"b": "x"}}"#, r#"{"a": []}"#] {
            assert!(
                matches!(
                    table_with_args(r#"{"a": {}}"#, refused),
                    Err(TableError::Json(_))
                ),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_table_the_closure_can_t_read_is_refused() {
        assert!(matches!(
            BuiltinGrantsTable::parse(
                r#"{"description": "", "cli": {"invokes": {"a": {}}}, "builtins": {"a": {}}, "args": {"a": null}}"#
            ),
            Err(TableError::CliInvokes(name)) if name == "a"
        ));
        // the grants and the args list the same builtins
        assert!(matches!(
            table_with_args(r#"{"a": {}}"#, "{}"),
            Err(TableError::ArgsUnmatched(name)) if name == "a"
        ));
        assert!(matches!(
            table_with_args(r#"{"a": {}}"#, r#"{"a": null, "b": null}"#),
            Err(TableError::ArgsUnmatched(name)) if name == "b"
        ));
        // a `when` names a boolean of the invoker's with a default
        for args in [
            r#"{"a": null, "b": null}"#,
            r#"{"a": {"x": null}, "b": null}"#,
        ] {
            assert!(
                matches!(
                    table_with_args(r#"{"a": {"invokes": {"b": {"when": "x"}}}, "b": {}}"#, args),
                    Err(TableError::WhenNotBoolean { builtin, invoked, when })
                        if builtin == "a" && invoked == "b" && when == "x"
                ),
                "{args}"
            );
        }
        assert!(matches!(
            table(r#"{"a": {"invokes": {"b": {}}}}"#),
            Err(TableError::UnknownInvoke { builtin, invoked }) if builtin == "a" && invoked == "b"
        ));
        // a `when` only on a builtin no other invokes, the command's
        assert!(table(r#"{"a": {"invokes": {"b": {"when": "x"}}}, "b": {}}"#).is_ok());
        assert!(matches!(
            table(r#"{"a": {"invokes": {"b": {}}}, "b": {"invokes": {"c": {"when": "x"}}}, "c": null}"#),
            Err(TableError::WhenBeyondCommand { builtin, invoked, when, invoker })
                if builtin == "b" && invoked == "c" && when == "x" && invoker == "a"
        ));
    }
}
