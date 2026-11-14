//! The builtin modules against their Node implementations: each case of
//! `tests/fixtures/host/cases.json` run under `fgro run` and held to what
//! the Node implementation printed for it, `expected.json`, which
//! `oracle.js` there writes under Node and Gro's own tests check against
//! Node, so neither side needs the other's runtime. A value that differs
//! by design is named in the case's `differs`, with the builtin's value
//! and why; everything else, the exit code and stderr included, is equal.
//!
//! Then every builtin in a patched realm (`tests/fixtures/host/patched/`):
//! a builtin's implementation runs at its first import, which can be after
//! user code replaced the intrinsics, so each row of the table is imported
//! for the first time, and each function it exports or reaches called, with
//! every intrinsic a program can reach a recording trap, and must be what
//! it is in a realm nothing patched and use no trap.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test's setup fails by panicking"
)]

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gro_runtime::resolver::BUILTINS;
use serde_json::Value;

use support::{Scratch, fgro_command, spawn};

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

/// The exports whose value is the process's own, by builtin, which the
/// patched and plain runs, two processes, show by type: the cases above
/// hold their values.
const PER_PROCESS: [(&str, &str); 1] = [("@fuzdev/gro/pid.ts", "pid")];

/// Property names trapped on the prototypes every value shares, beside the
/// names the implementations write: the ones a promise resolution, a
/// property definition, an iteration, a serialization, and an error read
/// from any object given to them.
const PROTOCOL_NAMES: [&str; 14] = [
    "then",
    "get",
    "set",
    "value",
    "writable",
    "enumerable",
    "configurable",
    "toJSON",
    "next",
    "done",
    "length",
    "name",
    "message",
    "cause",
];

/// The names trapped on the shared prototypes: every identifier a
/// builtin's implementation writes, read from its source, so a property it
/// reads or writes on an object that lacks it lands on a trap, and
/// [`PROTOCOL_NAMES`].
fn trapped_names() -> BTreeSet<String> {
    let implementations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gro_runtime/js/builtin");
    let mut names: BTreeSet<String> = PROTOCOL_NAMES.iter().map(ToString::to_string).collect();
    for builtin in BUILTINS {
        let source = std::fs::read_to_string(implementations.join(builtin.specifier)).unwrap();
        let words = source.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'));
        names.extend(
            words
                .filter(|word| !word.is_empty() && !word.starts_with(|c: char| c.is_ascii_digit()))
                .map(ToOwned::to_owned),
        );
        names.extend(builtin.exports.iter().map(ToString::to_string));
    }
    names
}

/// A project of the patched realm's fixtures with the two modules written
/// from the builtin table: `modules.js`, importing every row's namespace
/// with its exports, each marked when its value is the process's own, and
/// `names.js`, the names to trap.
fn patched_project(name: &str) -> Scratch {
    let fixtures = fixtures().join("patched");
    let fixture = |file: &str| std::fs::read_to_string(fixtures.join(file)).unwrap();
    let mut imports = Vec::new();
    let mut rows = Vec::new();
    for (i, builtin) in BUILTINS.iter().enumerate() {
        imports.push(format!("import * as m{i} from '{}';\n", builtin.specifier));
        let exports: Vec<Value> = builtin
            .exports
            .iter()
            .map(|export| {
                let per_process = PER_PROCESS.contains(&(builtin.specifier, export));
                serde_json::json!([export, per_process])
            })
            .collect();
        rows.push(format!(
            "\t[{}, {}, m{i}]",
            Value::from(builtin.specifier),
            Value::from(exports)
        ));
    }
    let modules = format!(
        "{}export const MODULES = [\n{}\n];\n",
        imports.concat(),
        rows.join(",\n")
    );
    let names = format!(
        "export const NAMES = {};\n",
        serde_json::to_string(&trapped_names()).unwrap()
    );
    let files = ["realm.js", "use.js", "patched.js", "plain.js", "control.js"].map(fixture);
    Scratch::new(
        name,
        &[
            ("package.json", "{\"type\": \"module\"}\n"),
            ("modules.js", &modules),
            ("names.js", &names),
            ("realm.js", &files[0]),
            ("use.js", &files[1]),
            ("patched.js", &files[2]),
            ("plain.js", &files[3]),
            ("control.js", &files[4]),
        ],
    )
}

/// What `fgro run` of `entry` in `project`, given two arguments, printed,
/// one line of JSON, having exited `0` with nothing on stderr.
fn run_entry(project: &Scratch, entry: &str) -> Value {
    let mut command = fgro_command(&["run", entry, "a", "b c"]);
    command.current_dir(&project.0);
    let output = spawn(command, b"");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stderr, "", "{entry}: {stdout}");
    assert_eq!(output.status.code(), Some(0), "{entry}: {stdout}");
    serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{entry}: {error}: {stdout}"))
}

/// Every row of the builtin table, imported for the first time in a realm
/// whose every reachable intrinsic is a trap, with each function it exports
/// called there, and each method of an object it exports or a call returns,
/// is what it is in a realm nothing patched: the same namespace, exports,
/// and results, shown property by property. And no trap recorded a use
/// while the implementations ran, at their top level, which the import
/// runs, or in a call. The rows come from the table, so a new one is
/// imported and called here without being listed; a function that declares
/// parameters isn't called, which fails here until `use.js` calls it with
/// arguments.
#[test]
fn every_builtin_is_the_same_in_a_patched_realm_and_uses_none_of_it() {
    for (specifier, export) in PER_PROCESS {
        let row = BUILTINS
            .iter()
            .find(|builtin| builtin.specifier == specifier);
        assert!(
            row.is_some_and(|row| row.exports.contains(&export)),
            "{specifier} {export}"
        );
    }
    let project = patched_project("patched");
    let plain = run_entry(&project, "plain.js");
    let patched = run_entry(&project, "patched.js");
    assert_eq!(plain["replaced"], 0);
    assert_eq!(plain["recorded"], "");
    // the intrinsics' own properties alone are thousands
    assert!(
        patched["replaced"].as_u64().unwrap() > 2000,
        "{}",
        patched["replaced"]
    );
    assert_eq!(patched["recorded"], "", "the traps an implementation used");
    assert_eq!(patched["lines"], plain["lines"]);
    for run in [&plain, &patched] {
        assert_eq!(
            run["uncalled"],
            serde_json::json!([]),
            "the functions `use.js` reached and didn't call"
        );
    }
    let lines: Vec<&str> = plain["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect();
    for builtin in BUILTINS {
        let specifier = builtin.specifier;
        let shown = |prefix: String| lines.iter().any(|line| line.starts_with(&prefix));
        assert!(shown(format!("{specifier}: object of null")), "{lines:?}");
        for export in builtin.exports {
            assert!(shown(format!("{specifier} {export}: ")), "{lines:?}");
        }
    }
    assert!(
        lines.iter().any(|line| line.contains("(): returned ")),
        "{lines:?}"
    );
}

/// The patched realm records each thing an implementation must not do,
/// done once in it while armed: a global read, a method of an intrinsic's
/// constructor and of its prototype, an iteration, a string made of an
/// object, `instanceof`, a species lookup, a read and a write of a property
/// an object lacks, a thenable lookup, and a property definition's read of
/// its descriptor. So a patched run that records nothing was watched.
#[test]
fn the_patched_realm_records_what_an_implementation_must_not_do() {
    let project = patched_project("control");
    let recorded = run_entry(&project, "control.js");
    let recorded: Vec<&str> = recorded
        .as_array()
        .unwrap()
        .iter()
        .map(|uses| uses.as_str().unwrap())
        .collect();
    // a property reached through more than one object is named for the
    // first that reached it, so each is matched by its key
    let expected = [
        "get globalThis.Object;",
        ".ownKeys;",
        ".map;",
        ".call;",
        ".Symbol(Symbol.iterator);",
        "get Object.prototype.Symbol(Symbol.toPrimitive);",
        "Symbol(Symbol.hasInstance);",
        "Symbol(Symbol.species);",
        "get Object.prototype.default;",
        "set Object.prototype.cwd;",
        "get Object.prototype.then;",
        "get Object.prototype.get;",
    ];
    assert_eq!(recorded.len(), expected.len(), "{recorded:?}");
    for (uses, expected) in recorded.iter().zip(expected) {
        assert!(uses.contains(expected), "{expected} in {uses}");
    }
}
