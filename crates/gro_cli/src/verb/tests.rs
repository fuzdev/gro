//! The verb's twins held to the fixtures Gro's TS writes
//! (`tests/fixtures/verb/`): the grant flags' split here, and the closure
//! of each command line it leaves, read from the carried table.

use std::collections::{BTreeMap, BTreeSet};

use gro_runtime::verb::closure::{
    ClosureBuiltin, ClosureFallback, ClosureGrants, ClosureProgram, CommandClosure, command_closure,
};
use gro_runtime::verb::table::{AllKeyword, BuiltinGrantsTable, BuiltinProgramGrant, PassEnv};
use serde_json::Value;

use crate::verb::grant_flags::split_grant_flags;

/// The fixture `name` of `tests/fixtures/verb/`.
pub fn read_fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/verb/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// A fixture's array of strings.
pub fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

/// Whether `value`'s fields are exactly `fields`, so a field the twin
/// doesn't read fails its test.
pub fn has_fields(value: &Value, fields: &[&str]) -> bool {
    let keys: BTreeSet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys == fields.iter().copied().collect()
}

/// A fixture's program, its role and its grant's fields as the table holds
/// them.
fn program(value: &Value) -> ClosureProgram {
    let field = |name| value.get(name);
    let known = ["role", "name", "local", "pass_env", "omit_env", "env"];
    let unknown: Vec<&String> = value
        .as_object()
        .unwrap()
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "a program's unknown fields: {unknown:?}"
    );
    ClosureProgram {
        role: value["role"].as_str().unwrap().to_owned(),
        grant: BuiltinProgramGrant {
            name: value["name"].as_str().unwrap().to_owned(),
            local: field("local").is_some_and(|local| local == &Value::Bool(true)),
            pass_env: field("pass_env").map(|pass_env| match pass_env {
                Value::String(all) if all == "all" => PassEnv::All(AllKeyword::All),
                names => PassEnv::Names(strings(names)),
            }),
            omit_env: field("omit_env").map(strings),
            env: field("env").map(|env| {
                env.as_object()
                    .unwrap()
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_owned()))
                    .collect::<BTreeMap<_, _>>()
            }),
        },
    }
}

/// A fixture's closure, for a command line Node runs.
fn closure(case: &Value) -> CommandClosure {
    let set = |value: &Value| strings(value).into_iter().collect::<BTreeSet<_>>();
    let grants = &case["grants"];
    assert!(has_fields(
        grants,
        &[
            "env",
            "fs",
            "programs",
            "project_modules",
            "stderr",
            "stdout"
        ]
    ));
    assert!(has_fields(&grants["fs"], &["read", "write"]));
    let flag = |name: &str| grants[name].as_bool().unwrap();
    let builtins = case["builtins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|builtin| {
            assert!(has_fields(builtin, &["check", "name"]));
            ClosureBuiltin {
                name: builtin["name"].as_str().unwrap().to_owned(),
                check: builtin["check"].as_bool().unwrap(),
            }
        })
        .collect();
    let grants = ClosureGrants {
        read: set(&grants["fs"]["read"]),
        write: set(&grants["fs"]["write"]),
        env: set(&grants["env"]),
        programs: grants["programs"]
            .as_array()
            .unwrap()
            .iter()
            .map(program)
            .collect(),
        project_modules: flag("project_modules"),
        stdout: flag("stdout"),
        stderr: flag("stderr"),
    };
    let fallback = case["fallback"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fallback| match fallback["kind"].as_str().unwrap() {
            "ungranted" => {
                assert!(has_fields(fallback, &["kind", "builtin"]));
                ClosureFallback::Ungranted {
                    builtin: fallback["builtin"].as_str().unwrap().to_owned(),
                }
            }
            "install" => {
                assert!(has_fields(fallback, &["kind"]));
                ClosureFallback::Install
            }
            "project_task" => {
                assert!(has_fields(fallback, &["kind"]));
                ClosureFallback::ProjectTask
            }
            kind => panic!("an unknown fallback: {kind}"),
        })
        .collect();
    CommandClosure {
        builtins,
        grants,
        fallback,
    }
}

#[test]
fn the_closure_is_node_s() {
    let table = BuiltinGrantsTable::carried().unwrap();
    let fixture = read_fixture("command_closure.json");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() > 1);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let split = split_grant_flags(&strings(&case["argv"])).unwrap();
        let actual = command_closure(&table, &split.cli_args);
        match case["outcome"].as_str().unwrap() {
            "runs" => {
                assert!(
                    has_fields(
                        case,
                        &["name", "argv", "outcome", "builtins", "grants", "fallback"]
                    ),
                    "{name}"
                );
                assert_eq!(actual, closure(case), "{name}");
            }
            // Node runs no builtin, and the closure, reading neither a
            // builtin's `Args` nor `--help`, is the command's
            outcome @ ("help" | "refused") => {
                let refused = case.get("refused").and_then(Value::as_str);
                if outcome == "help" {
                    assert!(has_fields(case, &["name", "argv", "outcome"]), "{name}");
                } else {
                    assert!(
                        has_fields(case, &["name", "argv", "outcome", "refused"]),
                        "{name}"
                    );
                }
                let command = &split.cli_args[0];
                let holds_command = actual
                    .builtins
                    .iter()
                    .any(|builtin| &builtin.name == command);
                match refused {
                    None | Some("forwarded_args" | "forwarded_help" | "invalid_args") => {
                        assert!(holds_command, "{name}");
                    }
                    // the args the CLI refuses here replace the positionals, naming none
                    Some("not_a_builtin" | "task_args") => {
                        assert!(
                            actual.builtins.is_empty() && actual.fallback.is_empty(),
                            "{name}"
                        );
                    }
                    Some(refused) => panic!("{name}: an unknown refusal: {refused}"),
                }
            }
            outcome => panic!("{name}: an unknown outcome: {outcome}"),
        }
    }
}
