//! A command's closure: the builtins a command line runs and the grants a
//! run of it is held to, or why it can't run on the binary.
//!
//! It's read from the grants table before any JS runs, the union of the
//! builtins' grants with the CLI's own row.
//!
//! The rules are the Node Gro's, each step's (`src/lib/task_args.ts`,
//! `invoke_task.ts`, `run_task.ts`), and `command_closure.json` holds them to
//! it (`src/test/command_closure_fixture.gen.ts` writes it from a reference
//! closure, whose comment says where each rule comes from):
//!
//! - The CLI's own row always.
//! - The command is the first positional of the args before any `--`, after
//!   the grant flags are split off: none is the listing or `--version`,
//!   `task` lists the project's tasks or, given a task file, runs a project
//!   task, whose grants no table holds ([`ClosureFallback::ProjectTask`]),
//!   and `run` with a path runs a script. A name the table doesn't hold runs
//!   nothing: Node refuses it.
//! - The command's args are its direct args with the last `-- gro <name>`
//!   section's merged over them (`argv.rs`). A flag keyed `_` reaches the
//!   positionals that name the command, as Node's parse has it.
//! - Each builtin's invokes join transitively. One marked `when` joins only
//!   when the command's args set that boolean arg; the table holds no
//!   `when` on a builtin another invokes ([`super::table::TableError`]).
//! - An invoke marked `check` drops the writes of the invoked builtin's
//!   grants, and of those it invokes in turn. The mark holds whatever the
//!   command line forwards, where on Node a section turning its check mode
//!   off (`-- gro gen --no-check`) runs it writing: the binary refuses what
//!   Node writes there.
//! - The command itself runs in check mode, its writes dropped the same way,
//!   when its args set `check` and the table marks it check-capable, some
//!   invoke marking it `check` (`gro gen --check`, `gro format --check`).
//! - A builtin that declares no grants means the command runs on the Node
//!   Gro ([`ClosureFallback::Ungranted`]), and so does `install` set in the
//!   command's args with the `npm_cli` program in the closure
//!   ([`ClosureFallback::Install`]).
//!
//! The closure reads no builtin's `Args`, so it doesn't know which args a
//! builtin refuses, or a `--help`: for those Node runs no builtin, and the
//! closure is the command's as though the args were valid, no more than the
//! valid command would get, and more than a refusal needs.
//!
//! Kind: the boundary for what grants a run gets, once a run is held to its
//! closure. The adversary is the project's code (its config, genfiles, and
//! the modules its tools load), which runs inside the run; the asset is
//! everything outside the union. It holds against deliberate code because
//! it's computed natively before any of that code runs; what it can't bound
//! is the union itself, which the command line chose.

use std::collections::BTreeSet;

use super::argv::{ArgvKeys, ParsedArgv, parse_argv};
use super::table::{BuiltinGrantsTable, BuiltinProgramGrant, BuiltinTaskGrants};

/// The command word that runs a project task by its path, or lists them.
pub const TASK_VERB: &str = "task";

/// The builtin that runs a script with Gro's loader, taking every arg after
/// its path as the script's.
pub const RUN_TASK_NAME: &str = "run";

/// The command word that marks a forwarded section as a Gro task's args.
const GRO_COMMAND: &str = "gro";

/// The program role whose closure runs an install, under `install`.
const NPM_CLI_ROLE: &str = "npm_cli";

/// The arg of the command that installs, with [`NPM_CLI_ROLE`] in its
/// closure.
const INSTALL_ARG: &str = "install";

/// The arg that runs a check-capable command in check mode.
const CHECK_ARG: &str = "check";

/// A command line's closure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandClosure {
    /// The builtins it runs, each in check mode or not, one reached both
    /// ways held both ways.
    pub builtins: BTreeSet<ClosureBuiltin>,
    /// The union of their grants and the CLI's own row.
    pub grants: ClosureGrants,
    /// Why it runs on the Node Gro, if it does, by the closure's own rules;
    /// a policy's fallback is the caller's.
    pub fallback: BTreeSet<ClosureFallback>,
}

/// A builtin in a closure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClosureBuiltin {
    /// Its name.
    pub name: String,
    /// Whether it runs in check mode, without its writes to the project.
    pub check: bool,
}

/// The union of a closure's grants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosureGrants {
    /// The directories read, relative to the project root.
    pub read: BTreeSet<String>,
    /// The directories written, relative to the project root.
    pub write: BTreeSet<String>,
    /// The environment variables read.
    pub env: BTreeSet<String>,
    /// The programs started, a role two builtins grant differently held
    /// each way.
    pub programs: BTreeSet<ClosureProgram>,
    /// Whether the project's modules load.
    pub project_modules: bool,
    /// Whether standard output is written.
    pub stdout: bool,
    /// Whether standard error is written.
    pub stderr: bool,
}

/// A program in a closure: its role and its grant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClosureProgram {
    /// The role it's granted by.
    pub role: String,
    /// The grant.
    pub grant: BuiltinProgramGrant,
}

/// Why a closure runs on the Node Gro.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClosureFallback {
    /// The builtin declares no grants, so nothing states what it reaches.
    Ungranted {
        /// The builtin.
        builtin: String,
    },
    /// The command installs packages, which needs the network.
    Install,
    /// The command runs a project task, whose grants no table holds.
    ProjectTask,
}

impl ClosureGrants {
    /// Adds `grants`, without their writes when `check`.
    fn add(&mut self, grants: &BuiltinTaskGrants, check: bool) {
        self.read.extend(grants.fs.read.iter().cloned());
        if !check {
            self.write.extend(grants.fs.write.iter().cloned());
        }
        self.env.extend(grants.env.iter().cloned());
        self.programs
            .extend(grants.programs.iter().map(|(role, grant)| ClosureProgram {
                role: role.clone(),
                grant: grant.clone(),
            }));
        self.project_modules |= grants.project_modules;
        self.stdout |= grants.stdout;
        self.stderr |= grants.stderr;
    }
}

/// The closure of the command line `cli_args`, the args after the CLI's own
/// path with the grant flags split off, read from `table`.
pub fn command_closure(table: &BuiltinGrantsTable, cli_args: &[String]) -> CommandClosure {
    let mut closure = CommandClosure::default();
    closure.grants.add(&table.cli, false);
    match to_command(cli_args) {
        Command::None => {}
        Command::ProjectTask => {
            closure.fallback.insert(ClosureFallback::ProjectTask);
        }
        Command::Builtin { name, args } => {
            if table.builtins.contains_key(&name) {
                let check = args.is_set(CHECK_ARG) && table.is_check_capable(&name);
                visit(table, &mut closure, &name, Some(&args), check);
                if args.is_set(INSTALL_ARG)
                    && closure
                        .grants
                        .programs
                        .iter()
                        .any(|program| program.role == NPM_CLI_ROLE)
                {
                    closure.fallback.insert(ClosureFallback::Install);
                }
            }
        }
    }
    closure
}

/// Adds the builtin `name` to `closure`, in check mode or not, and the
/// builtins it invokes; `args` are its args when it's the command, `None`
/// when another invokes it.
fn visit(
    table: &BuiltinGrantsTable,
    closure: &mut CommandClosure,
    name: &str,
    args: Option<&ArgvKeys>,
    check: bool,
) {
    let builtin = ClosureBuiltin {
        name: name.to_owned(),
        check,
    };
    if !closure.builtins.insert(builtin) {
        return;
    }
    let Some(Some(grants)) = table.builtins.get(name) else {
        closure.fallback.insert(ClosureFallback::Ungranted {
            builtin: name.to_owned(),
        });
        return;
    };
    closure.grants.add(grants, check);
    for (invoked, invoke) in &grants.invokes {
        // the table holds a `when` only on the command's invokes
        if let Some(when) = &invoke.when
            && !args.is_some_and(|args| args.is_set(when))
        {
            continue;
        }
        visit(table, closure, invoked, None, check || invoke.check);
    }
}

/// What a command line runs.
enum Command {
    /// No builtin and no project task: the listing, `--version`, or `gro
    /// task`'s listing.
    None,
    /// A project task, by its path.
    ProjectTask,
    /// The builtin `name`, or a name Node refuses, with its args, the
    /// forwarded section's merged over the direct ones.
    Builtin { name: String, args: ArgvKeys },
}

/// What `cli_args` run, as `to_task_args` and `invoke_cli` read them.
fn to_command(cli_args: &[String]) -> Command {
    // `gro run <path> ...` takes every arg after its path as the script's
    if cli_args.first().is_some_and(|first| first == RUN_TASK_NAME)
        && cli_args[1..]
            .iter()
            .take_while(|arg| *arg != "--")
            .any(|arg| !arg.starts_with('-'))
    {
        return Command::Builtin {
            name: RUN_TASK_NAME.to_owned(),
            args: ArgvKeys::default(),
        };
    }
    let (direct, rest) = split_direct(cli_args);
    let direct = parse_argv(direct);
    let Some(name) = first_positional(&direct) else {
        return Command::None;
    };
    let mut positionals = direct.positionals.into_iter().skip(1);
    if name == TASK_VERB {
        return match positionals.next() {
            Some(path) if is_task_path(&path) => Command::ProjectTask,
            _ => Command::None,
        };
    }
    let forwarded = to_forwarded_args(rest, &name).unwrap_or_default();
    Command::Builtin {
        args: direct.keys.merge(&forwarded),
        name,
    }
}

/// The command word of `cli_args`, as the closure reads it.
///
/// `cli_args` are the args after the CLI's own path with the grant flags
/// split off. The word is the first positional of the args before the first
/// `--`, as Node's parse reads them, none when there's none or a flag keyed
/// `_` replaced them. It may be a path, which names no builtin.
pub fn command_word(cli_args: &[String]) -> Option<String> {
    first_positional(&parse_argv(split_direct(cli_args).0))
}

/// `cli_args` split at the first `--`: the direct args, and the rest from
/// it on.
fn split_direct(cli_args: &[String]) -> (&[String], &[String]) {
    let forwarded_index = cli_args.iter().position(|arg| arg == "--");
    cli_args.split_at(forwarded_index.unwrap_or(cli_args.len()))
}

/// The first positional of `direct`, none when a flag keyed `_` replaced
/// the positionals.
fn first_positional(direct: &ParsedArgv) -> Option<String> {
    if direct.replaced {
        return None;
    }
    direct.positionals.first().cloned()
}

/// The keys of the last section of `rest`, the args from the first `--`,
/// that runs the builtin `name`, `-- gro <name> ...`, as
/// `to_forwarded_args_by_command` reads it. A malformed `gro` section makes
/// Node refuse the command line, so it's passed over.
fn to_forwarded_args(rest: &[String], name: &str) -> Option<ArgvKeys> {
    rest.split(|arg| arg == "--")
        .filter(|section| section.first().is_some_and(|first| first == GRO_COMMAND))
        .rev()
        .find_map(|section| {
            let parsed = parse_argv(&section[1..]);
            (!parsed.replaced && parsed.positionals.first().map(String::as_str) == Some(name))
                .then_some(parsed.keys)
        })
}

/// Whether `path` ends in a task file suffix, `.task.ts` or `.task.js`, as
/// Gro's `is_task_path` tests it.
pub fn is_task_path(path: &str) -> bool {
    path.ends_with(".task.ts") || path.ends_with(".task.js")
}

/// Whether a command word names a path rather than a builtin: it holds a
/// `/` or ends in a task file suffix, as Gro's `is_task_path_input` tests
/// it.
pub fn is_task_path_input(input: &str) -> bool {
    input.contains('/') || is_task_path(input)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{ClosureBuiltin, ClosureFallback, CommandClosure, command_closure};
    use crate::verb::table::BuiltinGrantsTable;

    /// A table of `builtins`, the JSON of each builtin's row by name, its CLI
    /// row reading `.`.
    fn table(builtins: &str) -> BuiltinGrantsTable {
        BuiltinGrantsTable::parse(&format!(
            r#"{{"description": "", "cli": {{"fs": {{"read": ["."]}}}}, "builtins": {builtins}}}"#
        ))
        .unwrap()
    }

    fn closure(table: &BuiltinGrantsTable, cli_args: &[&str]) -> CommandClosure {
        let cli_args: Vec<String> = cli_args.iter().map(|&arg| arg.to_owned()).collect();
        command_closure(table, &cli_args)
    }

    fn builtins(closure: &CommandClosure) -> Vec<(&str, bool)> {
        closure
            .builtins
            .iter()
            .map(|ClosureBuiltin { name, check }| (name.as_str(), *check))
            .collect()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    /// `a` invokes `b` in check mode, `b` invokes `c`, and `c` invokes `d`,
    /// each writing a directory of its own; `a` also invokes `c` writing.
    const CHAIN: &str = r#"{
        "a": {"fs": {"write": ["a"]}, "invokes": {"b": {"check": true}}},
        "b": {"fs": {"write": ["b"]}, "invokes": {"c": {}}},
        "c": {"fs": {"write": ["c"]}, "invokes": {"d": {}}},
        "d": {"fs": {"read": ["d"], "write": ["d"]}, "env": ["D"]},
        "e": {"fs": {"write": ["e"]}, "invokes": {"b": {"check": true}, "c": {}}}
    }"#;

    #[test]
    fn a_check_mark_drops_writes_transitively() {
        let table = table(CHAIN);
        let a = closure(&table, &["a"]);
        assert_eq!(
            builtins(&a),
            [("a", false), ("b", true), ("c", true), ("d", true)]
        );
        // what the marked builtins reach, but for their writes
        assert_eq!(a.grants.write, set(&["a"]));
        assert_eq!(a.grants.read, set(&[".", "d"]));
        assert_eq!(a.grants.env, set(&["D"]));
        // a builtin reached both ways is held both ways, writing
        let e = closure(&table, &["e"]);
        assert_eq!(
            builtins(&e),
            [
                ("b", true),
                ("c", false),
                ("c", true),
                ("d", false),
                ("d", true),
                ("e", false)
            ]
        );
        assert_eq!(e.grants.write, set(&["c", "d", "e"]));
        // the command itself is never in check mode
        assert_eq!(closure(&table, &["b"]).grants.write, set(&["b", "c", "d"]));
    }

    #[test]
    fn a_check_capable_command_s_own_check_drops_its_writes() {
        let table = table(CHAIN);
        let builtins_and_writes = |cli_args: &[&str]| {
            let closure = closure(&table, cli_args);
            (
                builtins(&closure)
                    .into_iter()
                    .map(|(name, check)| (name.to_owned(), check))
                    .collect::<Vec<_>>(),
                closure.grants.write,
            )
        };
        let checked = (
            vec![
                ("b".to_owned(), true),
                ("c".to_owned(), true),
                ("d".to_owned(), true),
            ],
            BTreeSet::new(),
        );
        // `b` is check-capable, `a` and `e` invoking it marked `check`
        assert_eq!(builtins_and_writes(&["b", "--check"]), checked);
        assert_eq!(
            builtins_and_writes(&["b", "--no-check", "--check"]),
            checked
        );
        assert_eq!(
            builtins_and_writes(&["b", "--", "gro", "b", "--check"]),
            checked
        );
        assert_eq!(
            builtins_and_writes(&["b", "--check", "--no-check"]).1,
            set(&["b", "c", "d"])
        );
        assert_eq!(
            builtins_and_writes(&["b", "--check", "--", "gro", "b", "--no-check"]).1,
            set(&["b", "c", "d"])
        );
        assert_eq!(
            builtins_and_writes(&["b", "--check=true"]).1,
            set(&["b", "c", "d"])
        );
        // `c` and `a` aren't: nothing marks an invoke of them `check`
        assert_eq!(builtins_and_writes(&["c", "--check"]).1, set(&["c", "d"]));
        assert_eq!(builtins_and_writes(&["a", "--check"]).1, set(&["a"]));
    }

    #[test]
    fn a_flag_keyed_underscore_names_the_command_as_node_s_positionals() {
        let table = table(
            r#"{"a": {"invokes": {"b": {"when": "x"}}}, "b": {"env": ["B"]}, "c": {"env": ["C"]}}"#,
        );
        let env = |cli_args: &[&str]| closure(&table, cli_args).grants.env;
        assert_eq!(env(&["--_=c"]), set(&["C"]));
        assert_eq!(env(&["--_", "c"]), set(&["C"]));
        assert_eq!(env(&["--_=a", "--x"]), set(&["B"]));
        assert_eq!(env(&["a", "--", "gro", "--_=a", "--x"]), set(&["B"]));
        // a boolean `_` replaces them, so no builtin is named, and its section is passed over
        assert_eq!(env(&["c", "-_"]), set(&[]));
        assert_eq!(env(&["--no-_", "c"]), set(&[]));
        assert_eq!(env(&["a", "--", "gro", "--no-_", "a", "--x"]), set(&[]));
        // `-_ a` gives `_` the value `a`: the later section runs `a`, without `--x`
        assert_eq!(
            env(&["a", "--", "gro", "a", "--x", "--", "gro", "-_", "a"]),
            set(&[])
        );
        // a replaced section, whose command line Node refuses, is passed over
        assert_eq!(
            env(&["a", "--", "gro", "a", "--x", "--", "gro", "a", "--no-_"]),
            set(&["B"])
        );
    }

    #[test]
    fn a_when_invoke_joins_as_the_command_s_args_set_it() {
        let table = table(
            r#"{
                "a": {"invokes": {"b": {"when": "x"}, "c": {}}},
                "b": {"fs": {"write": ["b"]}},
                "c": {}
            }"#,
        );
        let joins = |cli_args: &[&str]| {
            closure(&table, cli_args)
                .builtins
                .iter()
                .any(|builtin| builtin.name == "b")
        };
        assert!(!joins(&["a"]));
        assert!(joins(&["a", "--x"]));
        assert!(!joins(&["a", "--x", "--no-x"]));
        assert!(joins(&["a", "--no-no-x"]));
        // the last section forwarded to the command, merged over its args
        assert!(joins(&["a", "--", "gro", "a", "--x"]));
        assert!(!joins(&["a", "--x", "--", "gro", "a", "--no-x"]));
        assert!(!joins(&[
            "a", "--", "gro", "a", "--x", "--", "gro", "a", "--no-x"
        ]));
        assert!(joins(&[
            "a", "--", "gro", "a", "--no-x", "--", "gro", "a", "--x"
        ]));
        // a section forwarded to another command, or to a tool, isn't the command's
        assert!(!joins(&["a", "--", "gro", "c", "--x"]));
        assert!(!joins(&["a", "--", "vitest", "--x"]));
        // a value is no boolean's `true`
        assert!(!joins(&["a", "--x=true"]));
    }

    #[test]
    fn a_builtin_with_no_grants_and_an_install_run_on_node() {
        let table = table(
            r#"{
                "a": {"invokes": {"n": {}, "s": {"when": "install"}}},
                "n": null,
                "s": {"programs": {"npm_cli": {"name": "npm"}}},
                "t": {"programs": {"npm_cli": {"name": "npm"}}},
                "u": {"invokes": {"n": {}, "v": {"check": true}}},
                "v": {"invokes": {"n": {}}}
            }"#,
        );
        let ungranted = ClosureFallback::Ungranted {
            builtin: "n".to_owned(),
        };
        assert_eq!(
            closure(&table, &["a"]).fallback,
            BTreeSet::from([ungranted.clone()])
        );
        assert_eq!(
            closure(&table, &["a", "--install"]).fallback,
            BTreeSet::from([ClosureFallback::Install, ungranted.clone()])
        );
        // `install` without the npm role installs nothing; with it, it's the command's arg
        assert_eq!(closure(&table, &["n", "--install"]).fallback.len(), 1);
        assert_eq!(
            closure(&table, &["t", "--install"]).fallback,
            BTreeSet::from([ClosureFallback::Install])
        );
        assert!(closure(&table, &["t"]).fallback.is_empty());
        // reached in both modes, it's one fallback
        assert_eq!(
            closure(&table, &["u"]).fallback,
            BTreeSet::from([ungranted])
        );
    }

    #[test]
    fn a_command_line_names_its_command_as_node_reads_it() {
        let table = table(r#"{"a": {"env": ["A"]}, "run": null}"#);
        let cli_row = closure(&table, &[]);
        assert!(cli_row.builtins.is_empty() && cli_row.fallback.is_empty());
        assert_eq!(cli_row.grants.read, set(&["."]));
        // no command word, or a flag taking it as its value
        for cli_args in [
            &["--version"][..],
            &["--x", "a"],
            &["--", "gro", "a"],
            &["task"],
            &["task", "src/lib"],
            &["nope"],
            &["src/a.task.ts"],
        ] {
            assert_eq!(closure(&table, cli_args), cli_row, "{cli_args:?}");
        }
        assert_eq!(closure(&table, &["--x=1", "a"]).grants.env, set(&["A"]));
        assert_eq!(closure(&table, &["--no-x", "a"]).grants.env, set(&["A"]));
        for path in ["src/a.task.ts", "a.task.js"] {
            assert_eq!(
                closure(&table, &["task", path]).fallback,
                BTreeSet::from([ClosureFallback::ProjectTask])
            );
        }
        // a script's args are its own, whatever they spell
        let run = closure(&table, &["run", "x.ts", "--", "gro", "a"]);
        assert_eq!(builtins(&run), [("run", false)]);
    }
}
