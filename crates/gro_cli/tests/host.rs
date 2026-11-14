//! The builtin modules against their Node implementations: each case of
//! `tests/fixtures/host/cases.json` run under `fgro run` and held to what
//! the Node implementation printed for it, `expected.json`, which
//! `oracle.js` there writes under Node and Gro's own tests check against
//! Node, so neither side needs the other's runtime. A value that differs
//! by design is named in the case's `differs`, with the builtin's value
//! and why; everything else, the exit code and stderr included, is equal.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test's setup fails by panicking"
)]

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use gro_runtime::resolver::BUILTINS;
use serde_json::Value;

use support::{fgro_command, spawn};

fn fixtures() -> PathBuf {
    support::fixtures("host")
}

/// A fixture file, parsed as JSON.
fn read_json(file: &str) -> Value {
    let path = fixtures().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}. `node tests/fixtures/host/oracle.js --write` writes expected.json",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap()
}

/// A case of the table: its name, its host module, the arguments it's run
/// with, and the values that differ by design on `fgro`.
struct Case {
    name: String,
    module: String,
    args: Vec<String>,
    differs: Vec<Difference>,
}

/// A value of a case's output that differs by design.
struct Difference {
    /// Where, a JSON pointer into the output.
    at: String,
    /// The builtin's value there.
    fgro: Value,
    /// Why it differs.
    why: String,
}

/// The table's cases.
fn cases() -> Vec<Case> {
    let text = |value: &Value| value.as_str().expect("a string").to_owned();
    read_json("cases.json")
        .as_array()
        .expect("an array of cases")
        .iter()
        .map(|case| Case {
            name: text(&case["name"]),
            module: text(&case["module"]),
            args: case["args"]
                .as_array()
                .expect("an array of arguments")
                .iter()
                .map(text)
                .collect(),
            differs: case["differs"]
                .as_array()
                .expect("an array of differences")
                .iter()
                .map(|difference| Difference {
                    at: text(&difference["at"]),
                    fgro: difference["fgro"].clone(),
                    why: text(&difference["why"]),
                })
                .collect(),
        })
        .collect()
}

/// Sets the value at `pointer`, a JSON pointer, in `output`, adding the
/// key when its parent, an object, lacks it.
fn set_at(output: &mut Value, pointer: &str, value: Value) {
    let (parent, key) = pointer.rsplit_once('/').expect("a JSON pointer");
    let parent = output
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("no object at {parent}"));
    parent.insert(key.to_owned(), value);
}

/// Every row of the builtin table has a case, every case an expectation
/// and the reverse, and every difference names where, the builtin's value,
/// and why, and is one: its value isn't Node's.
#[test]
fn every_builtin_has_a_case_and_every_difference_is_one() {
    let cases = cases();
    let expected = read_json("expected.json");
    let modules: BTreeSet<String> = cases
        .iter()
        .map(|case| format!("@fuzdev/gro/{}.ts", case.module))
        .collect();
    let rows: BTreeSet<String> = BUILTINS
        .iter()
        .map(|builtin| builtin.specifier.to_owned())
        .collect();
    assert_eq!(modules, rows);
    let names: BTreeSet<&str> = cases.iter().map(|case| case.name.as_str()).collect();
    assert_eq!(names.len(), cases.len(), "a case's name is used twice");
    let expectations: BTreeSet<&str> = expected
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, expectations);
    for case in &cases {
        for difference in &case.differs {
            assert!(
                difference.at.starts_with('/') && !difference.why.is_empty(),
                "{}",
                case.name
            );
            assert_ne!(
                expected[&case.name]["output"].pointer(&difference.at),
                Some(&difference.fgro),
                "{}: {} doesn't differ from Node's, so it isn't a difference",
                case.name,
                difference.at
            );
        }
    }
}

/// Each case under `fgro run` exits with the code Node's did, prints
/// nothing on stderr, and prints what Node's did but for the case's named
/// differences.
#[test]
fn each_case_matches_the_node_implementation_but_for_its_named_differences() {
    let expected = read_json("expected.json");
    let started_in = fixtures().canonicalize().unwrap();
    for case in cases() {
        let mut args = vec!["run", "on_fgro.js", case.module.as_str()];
        args.extend(case.args.iter().map(|arg| {
            if arg == "{cwd}" {
                started_in.to_str().unwrap()
            } else {
                arg.as_str()
            }
        }));
        let mut command = fgro_command(&args);
        command.current_dir(&started_in);
        let output = spawn(command, b"");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stderr, "", "{}: {stdout}", case.name);
        let node = &expected[&case.name];
        assert_eq!(
            output.status.code().map(i64::from),
            node["code"].as_i64(),
            "{}",
            case.name
        );
        let printed: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|error| panic!("{}: {error}: {stdout}", case.name));
        let mut held_to = node["output"].clone();
        for difference in case.differs {
            set_at(&mut held_to, &difference.at, difference.fgro);
        }
        assert_eq!(
            printed, held_to,
            "{}: fgro's output, then Node's with the case's differences",
            case.name
        );
    }
}
