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
//! getter read), with every intrinsic a program can reach a reporting trap,
//! and must be what it is in a realm nothing patched and use no trap. A
//! run reports as it goes, a line of JSON for each thing, so a call that
//! ended the process would still have left what it used.
//!
//! And every builtin's first import made to fail (`tests/fixtures/host/forced/`):
//! an `import()` runs the implementation on its caller's stack, so a
//! caller that left it none cuts it short, and the import after must find
//! nothing of that run and make the module whole.

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

/// The uses of a hostile argument a builtin's function is known to make,
/// each as the patched realm reports it, `<call>: <use>`: a function that
/// reads, converts, or iterates what it's given runs the giver's code from
/// inside the builtin, so each use is listed here once it's been reviewed,
/// and any other fails the test. No builtin's function touches an argument.
const ARGUMENT_TOUCHES: [&str; 0] = [];

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
    let sources = PATCHED_FILES.map(fixture);
    let mut files = vec![
        ("package.json", "{\"type\": \"module\"}\n"),
        ("modules.js", modules.as_str()),
        ("names.js", names.as_str()),
    ];
    files.extend(
        PATCHED_FILES
            .into_iter()
            .zip(sources.iter().map(String::as_str)),
    );
    Scratch::new(name, &files)
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

    /// The run's last report, `done`, which a run that got to its end makes.
    fn done(&self) -> &Value {
        match self.0.last() {
            Some((kind, value)) if kind == "done" => value,
            last => panic!("the run ended before its end: {last:?}"),
        }
    }
}

/// `fgro run` of `entry` in `project`, given two arguments: its exit code,
/// what it reported, and its stderr.
fn run_entry(project: &Scratch, entry: &str) -> (Option<i32>, Reports, String) {
    let mut command = fgro_command(&["run", entry, "a", "b c"]);
    command.current_dir(&project.0);
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

/// Every row of the builtin table, imported for the first time in a realm
/// whose every reachable intrinsic is a trap, with everything it exports or
/// reaches exercised there (each function called with no arguments, with
/// hostile ones, and with a hostile receiver, each class constructed, each
/// getter read, and the same for what a call returns and what an object or
/// a prototype the builtin made holds), is what it is in a realm nothing
/// patched: the same namespace, exports, and outcomes, shown property by
/// property. And no trap reported a use while the implementations ran, at
/// their top level, which the import runs, or in a call, and no hostile
/// argument a use but the reviewed ones. The rows come from the table, so a
/// new one is imported and exercised here without being listed; a function
/// that declares parameters fails here until `use.js` has arguments for it,
/// and one that ends the process fails here for a run with no end, which
/// that row's test must then expect, with the reports the run left.
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
    let plain = run_whole(&project, "plain.js");
    let patched = run_whole(&project, "patched.js");
    assert_eq!(plain.done()["replaced"], 0);
    // the intrinsics' own properties alone are thousands
    let replaced = &patched.done()["replaced"];
    assert!(replaced.as_u64().unwrap() > 2000, "{replaced}");
    assert_eq!(
        patched.strings("trap"),
        [""; 0],
        "the traps an implementation used"
    );
    assert_eq!(patched.strings("line"), plain.strings("line"));
    for run in [&plain, &patched] {
        assert_eq!(
            run.strings("unexercised"),
            [""; 0],
            "the functions that declare parameters and `use.js` has no arguments for"
        );
        assert_eq!(
            run.done()["unused"],
            serde_json::json!([]),
            "the arguments `use.js` has for a function nothing reached"
        );
        assert_eq!(
            run.strings("touch"),
            ARGUMENT_TOUCHES,
            "the uses a builtin's function made of a hostile argument"
        );
    }
    let lines = plain.strings("line");
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
/// it's given an argument, a method and a getter of an exported object, a
/// method of a call's result, and a class's constructor, static method, and
/// getter, with a method that needs its own instance called on one; it
/// reports each use made of a hostile argument, of every kind, the
/// functions that declare parameters and were given no arguments, and the
/// arguments given for a function it never reached. So a patched run that
/// reports nothing was watched.
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
        } else {
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
    ] {
        let reported = reported(call);
        let touch = format!("touch {call}{touch}");
        assert!(reported.contains(&touch), "{touch}: {reported:?}");
    }
    // the arguments given for a function reach what no other call does, and touch nothing
    assert_eq!(
        reported("probe.coerces(arguments 1): returned string \"given\""),
        &[""; 0]
    );
    assert_eq!(
        reports.strings("unexercised"),
        ["probe.with_parameter", "probe.reads_option"]
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

/// A first import cut short is run again, whole, for every row of the
/// table: an `import()` of a builtin nothing has imported runs its
/// implementation on the caller's stack, so a caller at the stack's limit
/// makes it fail, with the host's report of a `RangeError` naming a frame
/// of the backing script, as often as it likes; and the import that is left
/// enough stack makes the module with the row's exports, the same module
/// every later import gets. So a failed first import leaves nothing a
/// later one finds: no builtin is lost to the run, for the project or for
/// a package that never failed one. A debug `deno_core` warns on stderr
/// that it couldn't read such a failure's stack, once each, and nothing
/// else is printed there.
#[test]
fn a_first_import_cut_short_is_run_again_whole() {
    let specifiers: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.specifier).collect();
    let modules = format!(
        "export const SPECIFIERS = {};\n",
        serde_json::to_string(&specifiers).unwrap()
    );
    let forced = std::fs::read_to_string(fixtures().join("forced/forced.js")).unwrap();
    let project = Scratch::new(
        "forced",
        &[
            ("package.json", "{\"type\": \"module\"}\n"),
            ("modules.js", &modules),
            ("forced.js", &forced),
        ],
    );
    let mut command = fgro_command(&["run", "forced.js", "a"]);
    command.current_dir(&project.0);
    let output = spawn(command, b"");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    let unexpected = stderr
        .lines()
        .find(|line| !line.starts_with("warning: Failed to create JsStackFrame from callsite"));
    assert_eq!(unexpected, None);
    let rows: Value =
        serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), BUILTINS.len());
    for (row, builtin) in rows.iter().zip(BUILTINS) {
        assert_eq!(row["specifier"], builtin.specifier);
        let rejections = row["rejections"].as_array().unwrap();
        assert!(!rejections.is_empty(), "{row}");
        for rejection in rejections {
            let rejection = rejection.as_str().unwrap();
            assert!(
                rejection.ends_with("RangeError: Maximum call stack size exceeded"),
                "{row}"
            );
        }
        // some failure names a frame of the backing script, so the import reached it
        assert_ne!(row["cut_at"], serde_json::json!([]), "{row}");
        let mut exports: Vec<&str> = builtin.exports.to_vec();
        exports.push("default");
        exports.sort_unstable();
        assert_eq!(row["exports"], serde_json::json!(exports), "{row}");
        assert_eq!(row["same"], true, "{row}");
    }
}
