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
//! for the first time, and everything it exports or reaches exercised (a
//! function called, with hostile arguments too, a class constructed, a
//! getter read, a setter given a value), with every intrinsic a program can
//! reach a reporting trap, and must be what it is in a realm nothing
//! patched and use no trap. A run reports as it goes, a line of JSON for
//! each thing, a call before it's made, so a call that ended the process,
//! as `exit`'s do, left which it was and what it used: the entry is run
//! again without that call, to a run that gets to its end, and each call
//! that ended one is held to the code it exited with.
//!
//! And every builtin's first import started short of stack
//! (`tests/fixtures/host/forced/`): an `import()` runs the implementation on
//! its caller's stack, so the loader refuses one with too little left before
//! anything of it runs, and a later import with enough makes the module
//! whole.

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
/// with, `fgro run`'s flags before the entry, the environment it's run in,
/// which holds nothing else, and the values that differ by design on
/// `fgro`.
struct Case {
    name: String,
    module: String,
    args: Vec<String>,
    flags: Vec<String>,
    env: Vec<(String, String)>,
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
            flags: case["flags"]
                .as_array()
                .map(|flags| flags.iter().map(text).collect())
                .unwrap_or_default(),
            env: case["env"]
                .as_object()
                .map(|env| {
                    env.iter()
                        .map(|(name, value)| (name.clone(), text(value)))
                        .collect()
                })
                .unwrap_or_default(),
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

/// Each case under `fgro run` with its flags, in an environment holding
/// the case's and nothing else, exits with the code Node's did, prints
/// nothing on stderr, and prints what Node's did but for the case's named
/// differences.
#[test]
fn each_case_matches_the_node_implementation_but_for_its_named_differences() {
    let expected = read_json("expected.json");
    let started_in = fixtures().canonicalize().unwrap();
    for case in cases() {
        let mut args = vec!["run"];
        args.extend(case.flags.iter().map(String::as_str));
        args.extend(["on_fgro.js", case.module.as_str()]);
        args.extend(case.args.iter().map(|arg| {
            if arg == "{cwd}" {
                started_in.to_str().unwrap()
            } else {
                arg.as_str()
            }
        }));
        let mut command = fgro_command(&args);
        command
            .env_clear()
            .envs(case.env.iter().map(|(name, value)| (name, value)))
            .current_dir(&started_in);
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
/// from any object given to them, the options a web class or a clone reads
/// from one, and the formatter the host looks up when an error's stack is
/// read. No list can hold every name an intrinsic reads from an object it's
/// given, and nothing can stand beneath `Object.prototype` to catch the
/// rest, so this is the names known to matter, not a proof.
const PROTOCOL_NAMES: [&str; 19] = [
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
    "return",
    "length",
    "name",
    "message",
    "cause",
    "fatal",
    "ignoreBOM",
    "transfer",
    "prepareStackTrace",
];

/// The indices trapped on the shared prototypes, where a read past an
/// array's end or of a hole lands, and a write of one an object lacks.
const TRAPPED_INDICES: std::ops::Range<u32> = 0..16;

/// The uses of a hostile argument a builtin's function may make, each
/// reviewed, by the function, as the patched realm names its calls, with
/// each use as it reports one: a function that reads, converts, or
/// iterates what it's given runs the giver's code from inside the builtin,
/// so each use is listed here once it's been reviewed, any other fails the
/// test, and so does one listed that no run made.
///
/// Only an error class's constructor makes any: it converts its kind and
/// its target to strings for the message when it's given none, and the
/// message when it is, and asks its options for a `cause`, as the Node
/// class does. What it runs is the caller's conversions and the caller's
/// options, and a function exported beside it runs none.
const ARGUMENT_USES: [(&str, &[&str]); 1] = [(
    "new @fuzdev/gro/env_error.ts EnvError",
    &[
        // a kind, a target, or a message converted to a string
        "proxy.get Symbol(Symbol.toPrimitive)",
        "proxy.get toString",
        "proxy.get valueOf",
        "callable.get Symbol(Symbol.toPrimitive)",
        "callable.get toString",
        "callable.get valueOf",
        "getters.Symbol(Symbol.toPrimitive)",
        "getters.toString",
        "getters.valueOf",
        "string[Symbol.toPrimitive]",
        "array[Symbol.toPrimitive]",
        // the options asked for a `cause`
        "proxy.has cause",
        "callable.has cause",
        "getters.cause",
        "string.cause",
        "array.cause",
    ],
)];

/// Asserts that each of `touches`, as the patched realm reports a use of a
/// hostile argument, `<call>: <use>`, is a use [`ARGUMENT_USES`] lists for
/// the function called, and, with `every`, that each one listed was made.
fn assert_argument_uses(touches: &[&str], every: bool) {
    let mut made = BTreeSet::new();
    let mut unreviewed = BTreeSet::new();
    for touch in touches {
        let listed = ARGUMENT_USES.iter().find_map(|(function, uses)| {
            let (_, used) = touch.strip_prefix(function)?.split_once("): ")?;
            uses.contains(&used).then_some((*function, used))
        });
        match listed {
            Some(listed) => {
                made.insert(listed);
            }
            None => {
                unreviewed.insert(*touch);
            }
        }
    }
    assert!(
        unreviewed.is_empty(),
        "the uses of a hostile argument no one reviewed: {unreviewed:#?}"
    );
    if every {
        for (function, uses) in ARGUMENT_USES {
            for used in uses {
                assert!(
                    made.contains(&(function, *used)),
                    "a reviewed use of a hostile argument no call made: {function}: {used}"
                );
            }
        }
    }
}

/// The calls that end the process, each as the patched realm names it,
/// with the code it exits with, in the order they're made: `exit` given
/// nothing, given the code `use.js` has for it, and borrowed by a hostile
/// receiver, which it never reads. Any other call that ends a run fails
/// the test, and so does one of these that returns.
const ENDING_CALLS: [(&str, i32); 3] = [
    ("@fuzdev/gro/exit.ts exit()", 0),
    ("@fuzdev/gro/exit.ts exit(arguments 1)", 7),
    ("@fuzdev/gro/exit.ts exit.call(proxy)", 0),
];

/// The names trapped on the shared prototypes: every identifier a
/// builtin's implementation writes, read from its source, so a property it
/// reads or writes on an object that lacks it lands on a trap,
/// [`PROTOCOL_NAMES`], and [`TRAPPED_INDICES`].
fn trapped_names() -> BTreeSet<String> {
    let implementations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gro_runtime/js/builtin");
    let mut names: BTreeSet<String> = PROTOCOL_NAMES.iter().map(ToString::to_string).collect();
    names.extend(TRAPPED_INDICES.map(|index| index.to_string()));
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

/// The patched realm's fixtures.
const PATCHED_FILES: [&str; 7] = [
    "realm.js",
    "exercise.js",
    "use.js",
    "patched.js",
    "plain.js",
    "control.js",
    "ended.js",
];

/// A project of the patched realm's fixtures with the modules the test
/// writes: from the builtin table, `modules.js`, importing every row's
/// namespace with its exports, each marked when its value is the process's
/// own, and `names.js`, the names to trap; and `skips.js`, the calls to
/// skip, none ([`write_skips`]).
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
    let sources = PATCHED_FILES.map(fixture);
    let skips = skips_module(&[]);
    let mut files = vec![
        ("package.json", "{\"type\": \"module\"}\n"),
        ("modules.js", modules.as_str()),
        ("names.js", names.as_str()),
        ("skips.js", skips.as_str()),
    ];
    files.extend(
        PATCHED_FILES
            .into_iter()
            .zip(sources.iter().map(String::as_str)),
    );
    Scratch::new(name, &files)
}

/// The module naming the calls a run skips, `ended`: the ones an earlier
/// run of the same entry ended the process in.
fn skips_module(ended: &[String]) -> String {
    format!(
        "export const ENDED = {};\n",
        serde_json::to_string(ended).unwrap()
    )
}

/// What a run of the patched realm's fixtures reported, in order: each line
/// of its stdout, `[kind, value]`.
struct Reports(Vec<(String, Value)>);

impl Reports {
    /// The values of the reports of `kind`, in order, each a string.
    fn strings(&self, kind: &str) -> Vec<&str> {
        let of_kind = self.0.iter().filter(|(reported, _)| reported == kind);
        of_kind
            .map(|(_, value)| value.as_str().expect("a string"))
            .collect()
    }

    /// The `line` reports, in order, with the path of `project`, the run's
    /// own, as `{project}`.
    fn lines(&self, project: &Scratch) -> Vec<String> {
        let dir = project.0.canonicalize().unwrap();
        let dir = dir.to_str().unwrap();
        let lines = self.strings("line").into_iter();
        lines.map(|line| line.replace(dir, "{project}")).collect()
    }

    /// The run's last report, `done`, which a run that got to its end makes.
    fn done(&self) -> &Value {
        match self.0.last() {
            Some((kind, value)) if kind == "done" => value,
            last => panic!("the run ended before its end: {last:?}"),
        }
    }

    /// Whether the run got to its end.
    fn is_whole(&self) -> bool {
        matches!(self.0.last(), Some((kind, _)) if kind == "done")
    }

    /// The call under way when the run ended, the last it reported
    /// starting, in a run that didn't get to its end.
    fn ended_in(&self) -> &str {
        let calls = self.strings("call");
        let last = calls.last().expect("a run that ended in a call");
        let line = format!("{last}: ");
        assert!(
            self.strings("line")
                .iter()
                .all(|shown| !shown.starts_with(&line)),
            "{last} returned, so the run ended past it"
        );
        last
    }
}

/// The grant that lets a test import every row, `env` among them, with
/// the variables [`run_entry`] sets and one named `hostile`, as a hostile
/// array's one name is.
const GRANT_ENV: &str = "--allow-env=GRO_PATCHED_SET,GRO_PATCHED_UNSET,hostile";

/// `fgro run` of `entry` in `project`, granted [`GRANT_ENV`], given two
/// arguments, in an environment holding only `GRO_PATCHED_SET`: its exit
/// code, what it reported, and its stderr.
fn run_entry(project: &Scratch, entry: &str) -> (Option<i32>, Reports, String) {
    let mut command = fgro_command(&["run", GRANT_ENV, entry, "a", "b c"]);
    command
        .env_clear()
        .env("GRO_PATCHED_SET", "set")
        .current_dir(&project.0);
    let output = spawn(command, b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let reports = stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{entry}: {error}: {line}"))
        })
        .collect();
    let stderr = String::from_utf8(output.stderr).unwrap();
    (output.status.code(), Reports(reports), stderr)
}

/// What a run of `entry` that got to its end reported, having exited `0`
/// with nothing on stderr.
fn run_whole(project: &Scratch, entry: &str) -> Reports {
    let (code, reports, stderr) = run_entry(project, entry);
    assert_eq!(stderr, "", "{entry}");
    assert_eq!(code, Some(0), "{entry}");
    reports.done();
    reports
}

/// What a run of `entry` reported once it got to its end, with each call
/// that ended an earlier run of it and the code that run exited with: a
/// run that ends in a call is run again with the call skipped, at most as
/// often as [`ENDING_CALLS`] has calls. No run printed anything on stderr,
/// and the last exited `0`.
fn run_past_ends(project: &Scratch, entry: &str) -> (Reports, Vec<(String, i32)>, Vec<Reports>) {
    let mut ended: Vec<(String, i32)> = Vec::new();
    let mut cut_short = Vec::new();
    loop {
        let skipped: Vec<String> = ended.iter().map(|(call, _)| call.clone()).collect();
        std::fs::write(project.0.join("skips.js"), skips_module(&skipped)).unwrap();
        let (code, reports, stderr) = run_entry(project, entry);
        assert_eq!(stderr, "", "{entry} after {ended:?}");
        let code = code.expect("the run exits, not killed by a signal");
        if reports.is_whole() {
            assert_eq!(code, 0, "{entry} after {ended:?}");
            return (reports, ended, cut_short);
        }
        assert!(
            ended.len() < ENDING_CALLS.len(),
            "{entry} ended in {}, after {ended:?}",
            reports.ended_in()
        );
        ended.push((reports.ended_in().to_owned(), code));
        cut_short.push(reports);
    }
}

/// Every row of the builtin table, imported for the first time in a realm
/// whose every reachable intrinsic is a trap, with everything it exports or
/// reaches exercised there (each function called with no arguments, with
/// hostile ones, with each argument `use.js` gives for it replaced in turn
/// by a hostile one, and with a hostile receiver, each class constructed,
/// each getter read, each setter given a value, and the same for what a
/// call returns or throws and what an object or a prototype the builtin
/// made holds), is what it is in a realm nothing patched: the same
/// namespace, exports, and outcomes, shown property by property. And no
/// trap reported a use while the implementations ran, at their top level,
/// which the import runs, or in a call, and no hostile argument a use but
/// the reviewed ones. The rows come from the table, so a new one is
/// imported and exercised here without being listed; a function it exports
/// or reaches fails here until `use.js` has an entry for it, with the
/// arguments it takes or none. A call that ends the process, as `exit`'s
/// do, is one of [`ENDING_CALLS`], with its code, in both realms, and the
/// run it cut short reported no trap and no touch before it ended; any
/// other call that ends a run fails here. Each run has a project of its own, so what a
/// builtin's function did to one's files the other doesn't find, and a
/// path under it is shown as `{project}`.
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
    let plain_dir = patched_project("plain");
    let patched_dir = patched_project("patched");
    let (plain, plain_ended, plain_cut_short) = run_past_ends(&plain_dir, "plain.js");
    let (patched, patched_ended, patched_cut_short) = run_past_ends(&patched_dir, "patched.js");
    let ending_calls: Vec<(String, i32)> = ENDING_CALLS
        .iter()
        .map(|(call, code)| ((*call).to_owned(), *code))
        .collect();
    assert_eq!(plain_ended, ending_calls);
    assert_eq!(patched_ended, ending_calls);
    assert_eq!(plain.done()["replaced"], 0);
    // the intrinsics' own properties alone are thousands
    let replaced = &patched.done()["replaced"];
    assert!(replaced.as_u64().unwrap() > 2000, "{replaced}");
    assert_eq!(
        patched.strings("trap"),
        [""; 0],
        "the traps an implementation used"
    );
    let lines = plain.lines(&plain_dir);
    assert_eq!(patched.lines(&patched_dir), lines);
    // the two projects are two directories, which a row that shows one tells apart
    assert_ne!(patched.strings("line"), plain.strings("line"));
    // a run a call ended used nothing before it did: its traps reported as they fired
    for run in plain_cut_short.iter().chain(&patched_cut_short) {
        assert_eq!(run.strings("trap"), [""; 0]);
        assert_argument_uses(&run.strings("touch"), false);
        assert_eq!(run.strings("unexercised"), [""; 0]);
    }
    for (call, _) in ENDING_CALLS {
        let skipped = format!("{call}: ended the process");
        assert!(lines.contains(&skipped), "{lines:?}");
    }
    for run in [&plain, &patched] {
        assert_eq!(
            run.strings("unexercised"),
            [""; 0],
            "the functions `use.js` has no entry for"
        );
        assert_eq!(
            run.done()["unused"],
            serde_json::json!([]),
            "the arguments `use.js` has for a function nothing reached"
        );
        // the uses a builtin's function made of a hostile argument
        assert_argument_uses(&run.strings("touch"), true);
    }
    for builtin in BUILTINS {
        let specifier = builtin.specifier;
        let shown = |prefix: String| lines.iter().any(|line| line.starts_with(&prefix));
        assert!(shown(format!("{specifier}: object of null")), "{lines:?}");
        for export in builtin.exports {
            assert!(shown(format!("{specifier} {export}: ")), "{lines:?}");
        }
    }
    // some row exports a function, called every way
    for call in ["(): returned ", "(proxy): ", ".call(proxy): "] {
        assert!(lines.iter().any(|line| line.contains(call)), "{lines:?}");
    }
}

/// The patched realm reports each thing an implementation must not do,
/// done once in it while armed: a global read, a method of an intrinsic's
/// constructor and of its prototype, an iteration, a string made of an
/// object, `instanceof`, a species lookup, a read and a write of a property
/// an object lacks, a thenable lookup, a property definition's read of its
/// descriptor, a read past an array's end, a web class's read of its
/// options, and the lookup of a stack's formatter. And the exerciser
/// reaches every place a builtin can do one from: a function only when
/// it's given an argument, a method, a getter, and a setter of an exported
/// object, a method of a call's result and of what a call threw, and a
/// class's constructor, static method, and getter, with a method that
/// needs its own instance called on one; it reports each call before it
/// makes it, and skips the ones it's told ended an earlier run; it reports each use made of a
/// hostile argument, of every kind, a callback's call, a fourth argument's,
/// and one past a check that only a given argument passes among them, the
/// functions it was given no entry for, and the entry given for a function
/// it never reached; and it reads no error's stack, which would use the
/// realm itself. So a patched run that reports nothing was watched. What a
/// call leaves to run later, a promise's reaction, runs after the traps are
/// disarmed: nothing here watches it.
#[test]
fn the_patched_realm_reports_what_an_implementation_must_not_do() {
    let project = patched_project("control");
    let reports = run_whole(&project, "control.js");
    // the reports after each `use`, by what the use is
    let mut uses: Vec<(String, Vec<&(String, Value)>)> = Vec::new();
    for report in &reports.0 {
        match (report.0.as_str(), uses.last_mut()) {
            ("use", _) => uses.push((report.1.to_string(), Vec::new())),
            (_, Some((_, after))) => after.push(report),
            (_, None) => panic!("{report:?} before any use"),
        }
    }
    // a property reached through more than one object is named for the
    // first that reached it, so each is matched by its key
    let expected = [
        "get globalThis.Object",
        ".ownKeys",
        ".map",
        ".call",
        ".Symbol(Symbol.iterator)",
        "get Object.prototype.Symbol(Symbol.toPrimitive)",
        "Symbol(Symbol.hasInstance)",
        "Symbol(Symbol.species)",
        "get Object.prototype.default",
        "set Object.prototype.cwd",
        "get Object.prototype.then",
        "get Object.prototype.get",
        "get Array.prototype.5",
        "get Object.prototype.fatal",
        ".prepareStackTrace",
    ];
    let (probe, done) = uses.split_last().unwrap();
    assert_eq!(done.len(), expected.len(), "{uses:?}");
    for ((_, after), expected) in done.iter().zip(expected) {
        let traps = after.iter().filter(|(kind, _)| kind == "trap");
        let mut traps = traps.map(|(_, trap)| trap.as_str().unwrap());
        assert!(
            traps.any(|trap| trap.contains(expected)),
            "{expected} in {after:?}"
        );
    }
    // the probe: what each call reported, the reports before its line
    assert_eq!(probe.0, "\"probe\"");
    let mut calls: Vec<(&str, Vec<String>)> = Vec::new();
    let mut before = Vec::new();
    for (kind, value) in &probe.1 {
        let value = value.as_str().unwrap_or_default();
        if kind == "line" {
            calls.push((value, std::mem::take(&mut before)));
        } else if kind != "call" {
            before.push(format!("{kind} {value}"));
        }
    }
    let reported = |call: &str| {
        let found = calls.iter().find(|(line, _)| line.starts_with(call));
        &found.unwrap_or_else(|| panic!("no {call} in {calls:?}")).1
    };
    let trap = "trap get globalThis.Object";
    for (call, uses_the_realm) in [
        ("probe.with_parameter(): returned", false),
        ("probe.with_parameter(proxy): returned", true),
        ("probe.with_parameter(getters): returned", true),
        ("probe.with_parameter(string): returned", true),
        ("probe.with_parameter(array): returned", true),
        ("probe.holder.method(): returned", true),
        ("get probe.holder.getter: returned", true),
        ("set probe.holder.written: returned", true),
        ("set probe.holder.written = proxy: returned", true),
        ("probe.throws(): threw", false),
        ("probe.throws() thrown.method(): returned", true),
        ("probe.returns_holder(): returned", false),
        ("probe.returns_holder().method(): returned", true),
        ("new probe.Handle(): returned", true),
        ("new probe.Handle(proxy): returned", true),
        ("new probe.Handle().read(): returned number 1", false),
        ("new probe.Handle().read.call(proxy): threw", false),
        ("probe.Handle.prototype.read(): threw", false),
        ("get new probe.Handle().size: returned", true),
        ("probe.Handle.open(): returned", true),
    ] {
        let reported = reported(call);
        assert_eq!(
            reported.iter().any(|use_| use_ == trap),
            uses_the_realm,
            "{call}: {reported:?}"
        );
    }
    for (call, touch) in [
        (
            "probe.coerces(proxy): ",
            "proxy.get Symbol(Symbol.toPrimitive)",
        ),
        (
            "probe.coerces(getters): ",
            "getters.Symbol(Symbol.toPrimitive)",
        ),
        ("probe.coerces(getters): ", "getters.toString"),
        ("probe.coerces(string): ", "string[Symbol.toPrimitive]"),
        (
            "probe.iterates(proxy): ",
            "proxy.get Symbol(Symbol.iterator)",
        ),
        (
            "probe.iterates(getters): ",
            "getters.Symbol(Symbol.iterator)",
        ),
        ("probe.iterates(array): ", "array[Symbol.iterator]"),
        ("probe.reads_option(proxy): ", "proxy.get recursive"),
        // past a check of the first argument, which only the given one passes
        (
            "probe.guarded(arguments 1, proxy at 1): ",
            "proxy.get recursive",
        ),
        // a callback, called
        ("probe.calls(callable): ", "callable.apply"),
        // a fourth argument, which a call of three hostile ones wouldn't pass
        (
            "probe.fourth(getters): ",
            "getters.Symbol(Symbol.toPrimitive)",
        ),
    ] {
        let reported = reported(call);
        let touch = format!("touch {call}{touch}");
        assert!(reported.contains(&touch), "{touch}: {reported:?}");
    }
    // an error's stack is the engine's accessor, which formats it with the realm: never read
    assert!(reported("probe.returns_error(): returned").is_empty());
    assert!(
        calls.iter().all(|(line, _)| !line.contains(".stack")),
        "{calls:?}"
    );
    // a setter that uses nothing reports nothing, whatever it's given: a hostile value is made
    // before the traps are armed
    for given in [
        "",
        " = proxy",
        " = callable",
        " = getters",
        " = string",
        " = array",
    ] {
        let call = format!("set probe.holder.kept{given}: returned");
        assert_eq!(reported(&call), &[""; 0], "{call}");
    }
    // a hostile argument's own report uses no trap: a proxy's trap of one argument has no key
    let enumerated = reported("probe.enumerates(proxy): ");
    assert!(
        enumerated
            .iter()
            .any(|use_| use_.ends_with("proxy.ownKeys")),
        "{enumerated:?}"
    );
    assert!(
        enumerated.iter().all(|use_| use_.starts_with("touch ")),
        "{enumerated:?}"
    );
    // the arguments given for a function reach what no other call does, and touch nothing
    assert_eq!(
        reported("probe.coerces(arguments 1): returned string \"given\""),
        &[""; 0]
    );
    // each call is reported before it's made, so the last reported is the one a run ended in
    let made = reports.strings("call");
    let shown: Vec<&str> = calls.iter().map(|(line, _)| *line).collect();
    assert!(made.len() > 50, "{made:?}");
    for (call, line) in made.iter().zip(
        shown
            .iter()
            .filter(|line| !line.ends_with(": ended the process")),
    ) {
        assert!(
            line.starts_with(&format!("{call}: ")),
            "{call} before {line}"
        );
    }
    // a call an earlier run ended in is skipped, and shown so, and the function's others are made
    assert_eq!(reported("probe.ended_a_run(): ended the process"), &[""; 0]);
    assert!(!made.contains(&"probe.ended_a_run()"), "{made:?}");
    assert!(reported("probe.ended_a_run(proxy): returned").contains(&trap.to_owned()));
    // a function with no entry, whether it declares a parameter or none
    assert_eq!(
        reports.strings("unexercised"),
        [
            "probe.with_parameter",
            "probe.reads_option",
            "probe.returns_holder"
        ]
    );
    assert_eq!(
        reports.done()["unused"],
        serde_json::json!(["probe.never_reached"])
    );
}

/// A trap's report is printed when the trap is used, never kept for the
/// run's end: a run that uses the realm once, armed, and then fails has
/// reported the use, though it never got to its end. So a builtin's
/// function that ends the process leaves what it used before it did.
#[test]
fn a_trap_reports_as_it_is_used() {
    let project = patched_project("ended");
    let (code, reports, stderr) = run_entry(&project, "ended.js");
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("ended"), "{stderr}");
    assert_eq!(reports.strings("trap"), ["get globalThis.Object"]);
    assert!(reports.0.iter().all(|(kind, _)| kind != "done"));
}

/// A first import short of stack is refused and run whole later, for every
/// row of the table, in a realm nothing patched and in one patched against
/// the host's report of a failure: an `import()` of a builtin nothing has
/// imported runs its implementation on the caller's stack, so the loader
/// refuses one with less stack left than that needs, before anything of it
/// runs. Swept up from near the stack's limit a frame at a time, every
/// import before the first that succeeds is refused, by the loader at least
/// once, with a `TypeError` whose `code` is `ERR_MODULE_NOT_FOUND`, as every
/// import that fails to resolve gets (nearest the limit, with the engine's
/// own `RangeError` when
/// `deno_core` can't make its handler on the refused import's promise), and
/// none is cut short partway, which would fail with the host's report of
/// the `RangeError` instead; the import that succeeds makes the module with
/// the row's exports, the same module every later import gets. Nothing is
/// printed on stderr, and the run ends with 0, by a refused import as by a
/// failed one: a refusal for want of stack is no refusal by rule. The sweep
/// starts the first import with every amount of stack left a frame apart, so
/// a build whose implementations need more than the loader leaves them
/// fails here.
#[test]
fn a_first_import_short_of_stack_is_refused_and_run_whole_later() {
    let specifiers: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.specifier).collect();
    let modules = format!(
        "export const SPECIFIERS = {};\n",
        serde_json::to_string(&specifiers).unwrap()
    );
    let read = |file: &str| std::fs::read_to_string(fixtures().join("forced").join(file)).unwrap();
    let (sweep, forced, hostile) = (read("sweep.js"), read("forced.js"), read("hostile.js"));
    let project = Scratch::new(
        "forced",
        &[
            ("package.json", "{\"type\": \"module\"}\n"),
            ("modules.js", &modules),
            ("sweep.js", &sweep),
            ("forced.js", &forced),
            ("hostile.js", &hostile),
        ],
    );
    for entry in ["forced.js", "hostile.js"] {
        let mut command = fgro_command(&["run", GRANT_ENV, entry]);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        // the status first: a run a signal ended has no code, and says so
        assert_eq!(
            output.status.code(),
            Some(0),
            "{entry}, {}: {stderr}",
            output.status
        );
        assert_eq!(stderr, "", "{entry}");
        let rows: Value =
            serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"));
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), BUILTINS.len(), "{entry}");
        for (row, builtin) in rows.iter().zip(BUILTINS) {
            assert_eq!(row["specifier"], builtin.specifier, "{entry}");
            assert_eq!(row["other"], serde_json::json!([]), "{entry}: {row}");
            assert!(
                row["refused"]["loader"].as_u64().unwrap() > 0,
                "{entry}: {row}"
            );
            let mut exports: Vec<&str> = builtin.exports.to_vec();
            exports.push("default");
            exports.sort_unstable();
            assert_eq!(row["exports"], serde_json::json!(exports), "{entry}: {row}");
            assert_eq!(row["same"], true, "{entry}: {row}");
        }
    }
}
