//! The builtin modules against their Node implementations: each case of
//! `tests/fixtures/host/cases.json` run under `fgro run` and held to what
//! the Node implementation printed for it, `expected.json`, which
//! `oracle.js` there writes under Node and Gro's own tests check against
//! Node, so neither side needs the other's runtime. A value that differs
//! by design is named in the case's `differs`, with the builtin's value
//! and why; everything else, the exit code and stderr included, is equal.
//!
//! Then every builtin in a patched realm (`tests/fixtures/host/patched/`):
//! a builtin's backing script runs at its first import, which can be after
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
//! (`tests/fixtures/host/forced/`): an `import()` runs the backing script on
//! its caller's stack, so one with too little left has the module built
//! from the event loop instead, whole, and every import of a builtin, in
//! whatever order its first ones interleave, gets the one module.
//!
//! And every builtin imported, deferred, and imported again: each import
//! gets the one module.
//!
//! And `fs`'s confinement, which no Node case can hold: every way out of a
//! handle and out of the run's roots refused, a special file stat'd without
//! waiting, a forged or borrowed handle refused, a polluted `then` adopted
//! by no record or handle a member settles with, and each refused reach failing the run, listed; the
//! rule that no remove or rename moves a root's directory or gains a
//! scope, and a copy's read judged by the handle's; and what no committed
//! tree or Node run can show: a FIFO's read refused at once, where Node's
//! waits, a name that isn't UTF-8, and listings holding no descriptor.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test's setup fails by panicking"
)]

mod support;

use std::collections::BTreeSet;
use std::fmt::Write as _;
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
/// which holds nothing else, the unix sockets that listen while it runs, by
/// path relative to the fixtures, which git can't hold, the directory it
/// writes in, the values that differ by design on `fgro`, and the lines
/// `fgro` prints for what its rules refused, which fail the run there,
/// where Node has no such rule.
struct Case {
    name: String,
    module: String,
    args: Vec<String>,
    flags: Vec<String>,
    env: Vec<(String, String)>,
    sockets: Vec<String>,
    scratch: Option<ScratchDir>,
    differs: Vec<Difference>,
    refused: Vec<String>,
}

/// A directory of the fixtures a case writes in, made empty but for its
/// symlinks before the case runs and removed after, as the oracle makes
/// it: its path relative to the fixtures, and each symlink's name and
/// target.
struct ScratchDir {
    dir: String,
    links: Vec<(String, String)>,
}

impl ScratchDir {
    /// Makes the directory, beneath `fixtures`, empty but for its links.
    fn make(&self, fixtures: &Path) {
        let at = fixtures.join(&self.dir);
        self.remove(fixtures);
        std::fs::create_dir(&at).unwrap();
        for (name, target) in &self.links {
            std::os::unix::fs::symlink(target, at.join(name)).unwrap();
        }
    }

    /// Removes the directory, beneath `fixtures`, and everything in it.
    fn remove(&self, fixtures: &Path) {
        let at = fixtures.join(&self.dir);
        if std::fs::symlink_metadata(&at).is_ok() {
            std::fs::remove_dir_all(&at).unwrap();
        }
    }
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
            refused: case["refused"]
                .as_array()
                .map(|lines| lines.iter().map(text).collect())
                .unwrap_or_default(),
            sockets: case["sockets"]
                .as_array()
                .map(|paths| paths.iter().map(text).collect())
                .unwrap_or_default(),
            scratch: case["scratch"].as_object().map(|scratch| ScratchDir {
                dir: text(&scratch["dir"]),
                links: scratch["links"]
                    .as_object()
                    .expect("an object of links")
                    .iter()
                    .map(|(name, target)| (name.clone(), text(target)))
                    .collect(),
            }),
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

/// Unix sockets listening at paths of the fixtures while held, each
/// removed when dropped, as the oracle makes a case's.
struct Sockets(Vec<(PathBuf, std::os::unix::net::UnixListener)>);

impl Sockets {
    /// A socket listening at each of `paths`, relative to `dir`, a socket an
    /// earlier run left there removed first.
    fn listen(dir: &Path, paths: &[String]) -> Self {
        use std::os::unix::fs::FileTypeExt as _;
        Self(
            paths
                .iter()
                .map(|path| {
                    let at = dir.join(path);
                    if std::fs::symlink_metadata(&at).is_ok_and(|meta| meta.file_type().is_socket())
                    {
                        std::fs::remove_file(&at).unwrap();
                    }
                    let listener = std::os::unix::net::UnixListener::bind(&at)
                        .unwrap_or_else(|error| panic!("{}: {error}", at.display()));
                    (at, listener)
                })
                .collect(),
        )
    }
}

impl Drop for Sockets {
    fn drop(&mut self) {
        for (at, _) in &self.0 {
            let _ = std::fs::remove_file(at);
        }
    }
}

/// Each case under `fgro run` with its flags, in an environment holding
/// the case's and nothing else, with its sockets listening and its scratch
/// directory made, exits with the
/// code Node's did, prints nothing on stderr, and prints what Node's did but
/// for the case's named differences; a case whose calls `fgro`'s rules
/// refuse prints the refusals it names on stderr instead, and turns Node's 0
/// into 1.
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
        let sockets = Sockets::listen(&started_in, &case.sockets);
        if let Some(scratch) = &case.scratch {
            scratch.make(&started_in);
        }
        let output = spawn(command, b"");
        if let Some(scratch) = &case.scratch {
            scratch.remove(&started_in);
        }
        drop(sockets);
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut refusals = String::new();
        for line in &case.refused {
            writeln!(refusals, "{line}").unwrap();
        }
        assert_eq!(stderr, refusals, "{}: {stdout}", case.name);
        let node = &expected[&case.name];
        let code = match node["code"].as_i64() {
            Some(0) if !case.refused.is_empty() => Some(1),
            code => code,
        };
        assert_eq!(output.status.code().map(i64::from), code, "{}", case.name);
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
/// names the builtins' scripts and members have: the ones a promise
/// resolution, a property definition, an iteration, a serialization, and an
/// error read from any object given to them, the options a web class or a
/// clone reads from one, the `prototype` a constructor reads of its
/// `new.target`, and the formatter the host looks up when an error's stack is
/// read. No list can hold every name an intrinsic reads from an object it's
/// given, and nothing can stand beneath `Object.prototype` to catch the rest,
/// so this is the names known to matter, not a proof.
const PROTOCOL_NAMES: [&str; 20] = [
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
    "prototype",
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
/// its target to strings for its message, and asks its options for a
/// `cause`, as the Node class does. What it runs is the caller's conversions and the caller's
/// options, and a function exported beside it runs none.
const ERROR_CLASS_USES: &[&str] = &[
    // a kind or a target converted to a string
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
];

/// [`ERROR_CLASS_USES`], by each error family's class.
const ARGUMENT_USES: [(&str, &[&str]); 2] = [
    ("new @fuzdev/gro/env_error.ts EnvError", ERROR_CLASS_USES),
    ("new @fuzdev/gro/fs_error.ts FsError", ERROR_CLASS_USES),
];

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

/// The names a builtin's backing script reads: each name the two texts of
/// `gro_runtime`'s `bootstrap/template.rs` destructure or read with `.`,
/// read from its source, so a name the template comes to read is trapped.
/// A destructuring this reader can't take apart, one that renames or
/// nests, fails here rather than trap a name no script reads.
fn script_names() -> BTreeSet<String> {
    let template =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../gro_runtime/src/bootstrap/template.rs");
    let source = std::fs::read_to_string(template).unwrap();
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let mut names = BTreeSet::new();
    for text in [
        "const WITH_ERROR: &str = \"",
        "const WITHOUT_ERROR: &str = \"",
    ] {
        let start = source.find(text).unwrap() + text.len();
        let body = &source[start..start + source[start..].find("\";").unwrap()];
        for line in body.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("const {") {
                let destructured = &rest[..rest.find('}').unwrap()];
                for name in destructured.split(',').map(str::trim) {
                    assert!(!name.is_empty() && name.chars().all(word), "{line}");
                    names.insert(name.to_owned());
                }
            }
            for (at, _) in line.match_indices('.') {
                let read: String = line[at + 1..].chars().take_while(|&c| word(c)).collect();
                if !read.is_empty() {
                    names.insert(read);
                }
            }
        }
    }
    assert!(names.contains("op_gro_namespace"), "{names:?}");
    names
}

/// The names trapped on the shared prototypes, so a property a builtin
/// reads or writes on an object that lacks it lands on a trap: the names
/// its backing script reads ([`script_names`]), every native member's name,
/// an export's or a method's, as `gro_runtime`'s
/// `tests/native_members.txt` pins the table, each row's exports and the
/// `default` every namespace has, [`PROTOCOL_NAMES`], and
/// [`TRAPPED_INDICES`].
fn trapped_names() -> BTreeSet<String> {
    let members =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../gro_runtime/tests/native_members.txt");
    let members = std::fs::read_to_string(members).unwrap();
    let mut names: BTreeSet<String> = PROTOCOL_NAMES.iter().map(ToString::to_string).collect();
    names.extend(TRAPPED_INDICES.map(|index| index.to_string()));
    names.extend(script_names());
    names.extend(
        members
            .lines()
            .map(|line| line.split(' ').nth(1).unwrap().to_owned()),
    );
    for builtin in BUILTINS {
        names.extend(builtin.exports.iter().map(ToString::to_string));
    }
    names.insert("default".to_owned());
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

/// A project of the patched realm's fixtures, with `w`, a directory a run
/// writes ([`GRANT_WRITE`]), and the modules the test writes: from the
/// builtin table, `modules.js`, importing every row's namespace with its
/// exports, each marked when its value is the process's own, and
/// `names.js`, the names to trap; and `skips.js`, the calls to skip, none
/// ([`write_skips`]).
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
        ("w/.keep", ""),
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

    /// Each trap the run reported, as `<call>: <trap>`, the call the one
    /// under way when it fired, the last reported starting before it, or
    /// none outside every call.
    fn traps(&self) -> Vec<String> {
        let mut call = "";
        let mut traps = Vec::new();
        for (kind, value) in &self.0 {
            let value = value.as_str().unwrap_or_default();
            match kind.as_str() {
                "call" => call = value,
                "trap" => traps.push(format!("{call}: {value}")),
                _ => {}
            }
        }
        traps
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

/// The traps a builtin's call may fire, each reviewed, by the call: a
/// promise fulfilled with an object makes V8 read the object's `then`,
/// through its prototypes, as the language's promise resolution does, which
/// no API skips. A read that fulfils with a `Uint8Array`, as the Node
/// module's does, reads `Object.prototype.then` so, once its work is done
/// and with none of the host's state borrowed, so a `then` a program set
/// there runs as it would on Node.
const SETTLED_THEN_READS: [&str; 2] = [
    "@fuzdev/gro/fs.ts fs.project.read(arguments 1): get Object.prototype.then",
    "@fuzdev/gro/fs.ts fs.project.read_if_exists(arguments 1): get Object.prototype.then",
];

/// Asserts that each of `traps`, as [`Reports::traps`] gives them, is one
/// [`SETTLED_THEN_READS`] lists, each listed one fired at most once, the
/// one read V8's resolution makes, so a read of the host's own in the same
/// call isn't hidden by it, and, with `every`, exactly once.
fn assert_reviewed_traps(traps: &[String], every: bool) {
    let unreviewed: Vec<&String> = traps
        .iter()
        .filter(|trap| !SETTLED_THEN_READS.contains(&trap.as_str()))
        .collect();
    assert_eq!(unreviewed, [&""; 0], "the traps a builtin used");
    for reviewed in SETTLED_THEN_READS {
        let fired = traps.iter().filter(|trap| *trap == reviewed).count();
        let expected = if every { 1..=1 } else { 0..=1 };
        assert!(
            expected.contains(&fired),
            "{reviewed} is listed, and fired {fired} times"
        );
    }
}

/// The grant that lets a test import every row, `env` among them, with
/// the variables [`run_entry`] sets and one named `hostile`, as a hostile
/// array's one name is.
const GRANT_ENV: &str = "--allow-env=GRO_PATCHED_SET,GRO_PATCHED_UNSET,hostile";

/// The grant to write `w` in the project [`patched_project`] makes, so a
/// handle on it writes and its writes get past their grant to the data and
/// options they read.
const GRANT_WRITE: &str = "--allow-write=w";

/// `fgro run` of `entry` in `project`, granted [`GRANT_ENV`] and
/// [`GRANT_WRITE`], given two arguments, in an environment holding only
/// `GRO_PATCHED_SET`: its exit code, what it reported, and its stderr.
fn run_entry(project: &Scratch, entry: &str) -> (Option<i32>, Reports, String) {
    let mut command = fgro_command(&["run", GRANT_ENV, GRANT_WRITE, entry, "a", "b c"]);
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
/// trap reported a use while the backing scripts ran, which the import
/// runs, or in a call, but the reviewed reads of a fulfilled value's `then`
/// ([`SETTLED_THEN_READS`]), and no hostile argument a use but the
/// reviewed ones. The rows come from the table, so a new one is
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
    assert_reviewed_traps(&patched.traps(), true);
    let lines = plain.lines(&plain_dir);
    assert_eq!(patched.lines(&patched_dir), lines);
    // the two projects are two directories, which a row that shows one tells apart
    assert_ne!(patched.strings("line"), plain.strings("line"));
    // a run a call ended used nothing before it did: its traps reported as they fired
    for run in plain_cut_short.iter().chain(&patched_cut_short) {
        assert_reviewed_traps(&run.traps(), false);
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

/// The patched realm reports each thing a builtin must not do, done once in
/// it while armed: a global read, a method of an intrinsic's constructor and
/// of its prototype, an iteration, a string made of an object, `instanceof`,
/// a species lookup, a read and a write of a property an object lacks, a read
/// of a member's name and of one a backing script reads where an object lacks
/// it, a thenable lookup, a property definition's read of its descriptor, a
/// read past an array's end, a web class's read of its options, and the
/// lookup of a stack's formatter. And the exerciser reaches every place a
/// builtin can do one from: a function only when it's given an argument, a
/// method, a getter, and a setter of an exported object, a method of a call's
/// result and of what a call threw, and a class's constructor, static method,
/// and getter, with a method that needs its own instance called on one; it
/// reports each call before it makes it, and skips the ones it's told ended
/// an earlier run; it reports each use made of a hostile argument, of every
/// kind, a callback's call, a fourth argument's, and one past a check that
/// only a given argument passes among them, the functions it was given no
/// entry for, and the entry given for a function it never reached; and it
/// reads no error's stack, which would use the realm itself. So a patched run
/// that reports nothing was watched. What a call leaves to run later, a
/// promise's reaction, runs after the traps are disarmed: nothing here
/// watches it.
#[test]
fn the_patched_realm_reports_what_a_builtin_must_not_do() {
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
        "get Object.prototype.narrow",
        "get Object.prototype.op_gro_namespace",
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

/// A project named `name` of the first-import fixtures
/// (`tests/fixtures/host/forced/`), with `modules.js` listing the table's
/// specifiers.
fn forced_project(name: &str) -> Scratch {
    let specifiers: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.specifier).collect();
    let modules = format!(
        "export const SPECIFIERS = {};\n",
        serde_json::to_string(&specifiers).unwrap()
    );
    let read = |file: &str| std::fs::read_to_string(fixtures().join("forced").join(file)).unwrap();
    let sources = FORCED_FILES.map(read);
    let mut files = vec![
        ("package.json", "{\"type\": \"module\"}\n"),
        ("w/.keep", ""),
        ("modules.js", modules.as_str()),
    ];
    files.extend(
        FORCED_FILES
            .into_iter()
            .zip(sources.iter().map(String::as_str)),
    );
    Scratch::new(name, &files)
}

/// The files of `tests/fixtures/host/forced/`.
const FORCED_FILES: [&str; 5] = [
    "sweep.js",
    "forced.js",
    "hostile.js",
    "race.js",
    "static_env_error.js",
];

/// `fgro run` of `entry` in `project`, granted [`GRANT_ENV`], in an empty
/// environment: what it printed, parsed as JSON, once it exited `0` with
/// nothing on stderr.
fn run_printing(project: &Scratch, entry: &str) -> Value {
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
    serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{entry}: {error}: {stdout}"))
}

/// A first import short of stack runs whole, for every row of the table, in
/// a realm nothing patched and in one patched against the host's report of
/// a failure: an `import()` of a builtin nothing has imported runs its
/// backing script on the caller's stack, so with less stack left than that
/// needs `deno_core` builds the module from the event loop instead. Swept
/// up from near the stack's limit a frame at a time, in one turn, every
/// import gets the module, with the row's exports, the same module a later
/// import gets, but nearest the limit, where the engine's own `RangeError`
/// rejects an import whose promise `deno_core` can't make its handler on.
/// Some settle after imports started above them, which built the module on
/// the spot, so those were built from the event loop; none is cut short
/// partway, which would fail with the host's report of the `RangeError`.
/// Nothing is printed on stderr, and the run ends with 0. The sweep starts
/// the first import with every amount of stack left a frame apart, so a
/// build whose builtins need more than the loader leaves them fails here.
#[test]
fn a_first_import_short_of_stack_is_built_from_the_event_loop() {
    let project = forced_project("forced");
    for entry in ["forced.js", "hostile.js"] {
        let rows = run_printing(&project, entry);
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), BUILTINS.len(), "{entry}");
        for (row, builtin) in rows.iter().zip(BUILTINS) {
            assert_eq!(row["specifier"], builtin.specifier, "{entry}");
            assert_eq!(row["other"], serde_json::json!([]), "{entry}: {row}");
            assert!(row["late"].as_u64().unwrap() > 0, "{entry}: {row}");
            let mut exports: Vec<&str> = builtin.exports.to_vec();
            exports.push("default");
            exports.sort_unstable();
            assert_eq!(row["exports"], serde_json::json!(exports), "{entry}: {row}");
            assert_eq!(row["same"], true, "{entry}: {row}");
        }
    }
}

/// Every import of a builtin gets the one module, however its first
/// imports interleave in a turn (`race.js`): ones deep enough in the stack
/// that the module is built from the event loop and one near the top, which
/// builds it on the spot, in either order and three times over; deep ones
/// twice and no shallow one; deep ones and a module importing the builtin
/// statically, loaded in the same turn; a builtin whose module holds the
/// run's handle; and source-phase imports, refused deep and shallow alike,
/// before deep and shallow ones. A host-made class and a handle are the same through
/// every namespace, and in each case where a shallow import started after
/// deep ones, some deep ones settled after it, built from the event loop.
#[test]
fn every_import_of_a_builtin_in_a_turn_gets_one_module() {
    let project = forced_project("race");
    let rows = run_printing(&project, "race.js");
    let rows = rows.as_array().unwrap();
    for row in rows {
        assert_eq!(row["same"], true, "{row}");
        assert_eq!(row["errors"], serde_json::json!([]), "{row}");
        if let Some(late) = row["late"].as_u64() {
            assert!(late > 0, "{row}");
        }
    }
    let case = |name: &str| {
        rows.iter()
            .find(|row| row["case"] == name)
            .unwrap_or_else(|| panic!("{name}: {rows:?}"))
    };
    assert_eq!(case("deep, static")["classes"], true);
    assert_eq!(case("deep, shallow, a handle")["handles"], true);
    assert_eq!(
        case("source phase, then deep, shallow")["sources"],
        serde_json::json!({"imported": 0, "errors": ["ReferenceError"]})
    );
    assert_eq!(rows.len(), 7, "{rows:?}");
}

/// Every import of a builtin gets one module, an `import.defer()` between
/// them included: for every row of the table, `import()`, then
/// `import.defer()`, then `import()` again, each awaited, and the last
/// import gets the first one's namespace, and the deferred one the same
/// values. `deno_core` answers only an evaluation-phase `import()` of a
/// module it built already from its map, so the deferred import loads the
/// builtin again, and that load builds no second module of it (`deno_core`
/// patched, `vendor/README.md`), which the next import would have got.
#[test]
fn an_import_defer_between_imports_of_a_builtin_gets_its_one_module() {
    let specifiers: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.specifier).collect();
    let main = format!(
        "const rows = [];
for (const specifier of {}) {{
\tconst first = await import(specifier);
\tconst deferred = await import.defer(specifier);
\tconst later = await import(specifier);
\tconst values = Object.keys(first).every((key) => deferred[key] === first[key]);
\trows.push({{ specifier, same: later === first, values }});
}}
console.log(JSON.stringify(rows));
",
        serde_json::to_string(&specifiers).unwrap()
    );
    let project = Scratch::new(
        "deferred",
        &[
            ("package.json", "{\"type\": \"module\"}\n"),
            ("main.js", &main),
        ],
    );
    let rows = run_printing(&project, "main.js");
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), BUILTINS.len(), "{rows:?}");
    for (row, builtin) in rows.iter().zip(BUILTINS) {
        assert_eq!(row["specifier"], builtin.specifier, "{row}");
        assert_eq!(row["same"], true, "{row}");
        assert_eq!(row["values"], true, "{row}");
    }
}

/// The script [`fs_refuses_every_escape_and_forgery`] runs: each call's
/// outcome, as a JSON line, the project's path as `{project}`.
const FS_ESCAPES: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { create_hasher } from '@fuzdev/gro/blake3.ts';
const project = fs.project.path;
const outside = process_outside();
function process_outside() {
\treturn project.replace(/[^/]+$/, 'fs_outside_of_it');
}
const show = (value) => {
\tif (value === null || typeof value !== 'object') return typeof value === 'string' ? value.replace(project, '{project}') : value;
\tif (typeof value.path === 'string') return { handle: value.path.replace(project, '{project}') };
\treturn { kind: value.kind };
};
const how = async (call) => {
\tlet value;
\ttry {
\t\tvalue = call();
\t} catch (error) {
\t\treturn ['threw', error.constructor.name, error.kind ?? null];
\t}
\tif (!(value instanceof Promise)) return ['returned', show(value)];
\ttry {
\t\treturn ['fulfilled', show(await value)];
\t} catch (error) {
\t\treturn ['rejected', error.constructor.name, error.kind ?? null];
\t}
};
const dir = fs.project;
const read_prototype = Object.getPrototypeOf(Object.getPrototypeOf(dir));
const calls = {
\tdot_dot: () => dir.stat('../fs_outside_of_it/f.txt'),
\tdot_dot_back_in: () => dir.stat(`../${project.split('/').pop()}/package.json`),
\tabsolute: () => dir.stat('/etc/hostname'),
\tabsolute_link: () => dir.stat('to_outside/f.txt'),
\trelative_link: () => dir.stat('climbing'),
\tproc_link: () => dir.stat('to_proc'),
\tproc_dir_link: () => dir.stat('to_proc_dir/status'),
\tlink_dir: () => dir.dir('to_outside'),
\tlink_dir_sync: () => dir.stat_sync('to_outside', { follow: false }),
\tfifo: () => dir.stat('fifo'),
\tfifo_exists: () => dir.exists_sync('fifo'),
\tfifo_dir: () => dir.dir('fifo'),
\tclone: () => read_prototype.stat.call(structuredClone(dir), 'package.json'),
\tlook_alike: () => read_prototype.stat.call(Object.create(Object.getPrototypeOf(dir)), 'package.json'),
\twrong_kind: () => read_prototype.stat.call(create_hasher(), 'package.json'),
\tread_only_writes: () => Object.getPrototypeOf(dir).write.call(dir.readonly(), 'x', 'y'),
\topen_outside: () => fs.open_dir(outside),
\topen_missing_outside: () => fs.open_dir(`${outside}/missing`),
\topen_dot_dot: () => fs.open_dir(`${project}/../fs_outside_of_it`),
\topen_link_out: () => fs.open_dir(`${project}/to_outside`),
\treal_path_link_out: () => fs.real_path(`${project}/to_outside/f.txt`),
\twrite: () => dir.write('new.txt', 'data'),
};
const ended = {};
for (const [label, call] of Object.entries(calls)) ended[label] = await how(call);
Object.prototype.then = (resolve) => resolve('hijacked');
ended.polluted_then = [
\tawait dir.stat('package.json').then((info) => info.kind),
\tawait fs.open_dir(project).then((opened) => opened.path === project),
\tawait dir.exists('package.json').then((found) => found),
];
delete Object.prototype.then;
console.log(JSON.stringify(ended));
";

/// `fs` on a project with symlinks out of it, absolute and climbing, one
/// into `/proc`, a FIFO, and a directory beside it: every escape from a
/// handle is `outside_root` and every reach past the project's root
/// `not_granted`, recorded and failing the run, listed in order, a
/// symlink out of the root among them, the hints naming each flag and no
/// path, a write through `fs.project` the directory to open after its
/// grant, never the project's; a FIFO is stat'd as `other` without
/// waiting for a writer, and isn't a directory; a cloned, look-alike,
/// other row's, or read-only handle is refused before anything is read;
/// and a polluted `Object.prototype.then` is adopted by no record or handle
/// a member settles with.
#[test]
fn fs_refuses_every_escape_and_forgery() {
    let project = Scratch::new(
        "fs_escapes",
        &[
            // named, so it's no type marker and the project's root is here
            (
                "package.json",
                "{\"name\": \"fs_escapes\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_ESCAPES),
        ],
    );
    let outside = project.0.with_file_name("fs_outside_of_it");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("f.txt"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, project.0.join("to_outside")).unwrap();
    std::os::unix::fs::symlink("../fs_outside_of_it/f.txt", project.0.join("climbing")).unwrap();
    std::os::unix::fs::symlink("/proc/self/status", project.0.join("to_proc")).unwrap();
    std::os::unix::fs::symlink("/proc/self", project.0.join("to_proc_dir")).unwrap();
    let fifo = std::process::Command::new("mkfifo")
        .arg(project.0.join("fifo"))
        .status()
        .unwrap();
    assert!(fifo.success());
    let mut command = fgro_command(&["run", "main.js"]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    let _ = std::fs::remove_dir_all(&outside);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let root = project.0.display().to_string();
    let out = outside.display().to_string();
    assert_eq!(
        stderr,
        format!(
            "error: fs refused a read: not_granted: {out}\n\
             error: fs refused a read: not_granted: {out}/missing\n\
             error: fs refused a read: not_granted: {root}/../fs_outside_of_it\n\
             error: fs refused a read: not_granted: {root}/to_outside\n\
             error: fs refused a read: not_granted: {root}/to_outside/f.txt\n\
             error: fs refused a write: not_granted: {root}/new.txt\n\
             error: a refusal fails the run, whether or not the program caught it\n\
             {HINT_READ}\n\
             {HINT_WRITE}\n"
        ),
        "{stdout}"
    );
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let ended: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "dot_dot": ["rejected", "FsError", "outside_root"],
            "dot_dot_back_in": ["rejected", "FsError", "outside_root"],
            "absolute": ["rejected", "FsError", "outside_root"],
            "absolute_link": ["rejected", "FsError", "outside_root"],
            "relative_link": ["rejected", "FsError", "outside_root"],
            "proc_link": ["rejected", "FsError", "outside_root"],
            "proc_dir_link": ["rejected", "FsError", "outside_root"],
            "link_dir": ["threw", "FsError", "outside_root"],
            "link_dir_sync": ["returned", {"kind": "symlink"}],
            "fifo": ["fulfilled", {"kind": "other"}],
            "fifo_exists": ["returned", true],
            "fifo_dir": ["threw", "FsError", "not_a_directory"],
            "clone": ["rejected", "TypeError", null],
            "look_alike": ["rejected", "TypeError", null],
            "wrong_kind": ["rejected", "TypeError", null],
            "read_only_writes": ["rejected", "TypeError", null],
            "open_outside": ["rejected", "FsError", "not_granted"],
            "open_missing_outside": ["rejected", "FsError", "not_granted"],
            "open_dot_dot": ["rejected", "FsError", "not_granted"],
            "open_link_out": ["rejected", "FsError", "not_granted"],
            "real_path_link_out": ["rejected", "FsError", "not_granted"],
            "write": ["rejected", "FsError", "not_granted"],
            "polluted_then": ["file", true, true],
        })
    );
}

/// `fs.project` is the entry's project root, never the working directory:
/// a run started in a subdirectory of the project, of an entry above it,
/// reads from the project's root, and the working directory is a string
/// with no authority.
#[test]
fn fs_project_is_the_entry_s_root_not_the_working_directory() {
    let project = Scratch::new(
        "fs_project",
        &[
            (
                "package.json",
                "{\"name\": \"fs_project\", \"type\": \"module\"}\n",
            ),
            (
                "main.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 import { cwd } from '@fuzdev/gro/cwd.ts';\n\
                 console.log(JSON.stringify([fs.project.path, cwd(), await fs.project.exists('main.js')]));\n",
            ),
            ("sub/inner.txt", ""),
        ],
    );
    let mut command = fgro_command(&["run", "../main.js"]);
    command.env_clear().current_dir(project.0.join("sub"));
    let output = spawn(command, b"");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!([project.0.display().to_string(), project.path("sub"), true])
    );
}

/// The script [`fs_roots_by_flag_scope_their_handles`] runs: each reach
/// through the roots the flags grant, as a JSON line, given the read root's
/// real path, the write root's, and the read root's path through a symlink.
const FS_ROOTS: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const [r, w, l] = args;
const p = fs.project.path;
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message];
\t}
};
const first = async (listing) => (await listing.next()).value?.name ?? null;
const ended = {
\tread_by_alias: await how(async () => (await fs.open_dir(l)).read_text('f.txt')),
\talias_is_real: await how(async () => (await fs.open_dir(`${l}/`)).path === r),
\treal_path_by_alias: await how(async () => (await fs.real_path(`${l}/f.txt`)) === `${r}/f.txt`),
\tread_by_real: await how(async () => (await fs.open_dir(r)).dir('sub').read_text('g.txt')),
\tread_root_writes: await how(async () => (await fs.open_dir(r)).write('x.txt', 'x')),
\twrite_only_reads: await how(async () => (await fs.open_dir(w)).read_text('f.txt')),
\twrite_only_stats: await how(async () => (await fs.open_dir(w)).stat('f.txt')),
\twrite_only_lists: await how(async () => first((await fs.open_dir(w)).list())),
\twrite_only_writes: await how(async () => (await fs.open_dir(w)).write('x.txt', 'x')),
\twrite_only_sub_writes: await how(async () => (await fs.open_dir(w)).dir('sub').mkdir('x')),
\twrite_only_sub_reads: await how(async () => (await fs.open_dir(w)).dir('sub').exists('g.txt')),
\twrite_only_readonly: await how(async () => (await fs.open_dir(w)).readonly().exists('f.txt')),
\twrite_only_real_path: await how(() => fs.real_path(`${w}/f.txt`)),
\tinner_write_reads: await how(async () => (await fs.open_dir(`${p}/out`)).read_text('f.txt')),
\tinner_write_writes: await how(async () => (await fs.open_dir(`${p}/out`)).remove('f.txt')),
\tinner_write_climbs: await how(async () => (await fs.open_dir(`${p}/out/up/sub`)).path === `${p}/sub`),
\tproject_dir_writes: await how(() => fs.project.dir('out').write('x.txt', 'x')),
\tproject_writes: await how(() => fs.project.write('x.txt', 'x')),
\tleaves_its_root: await how(() => fs.open_dir(`${r}/abs`)),
\tabsolute_link_in_project: await how(() => fs.open_dir(`${p}/to_r`)),
};
console.log(JSON.stringify(ended));
";

/// The roots `--allow-read=` and `--allow-write=` grant, beside the
/// project's: a read root by a path through a symlink, which matches by
/// that path and its real one alike, the handle named by its real path; a
/// root only written, whose handles write and narrow, each write landing
/// where it names, but read nothing, `fs.real_path` included; a relative
/// write root inside the project, read as the project is, and reaching
/// the project through a symlink out of it; and
/// `fs.project`, whose handles never write, whatever root inside it is
/// written. The flag through a symlink is noted with its real path as the
/// run starts. Every refusal is listed with the kind of grant it needed
/// and fails the run, and the hints name each flag a refusal needed and,
/// for a write a root already grants, a handle to open instead, never a
/// path.
#[test]
fn fs_roots_by_flag_scope_their_handles() {
    let project = Scratch::new(
        "fs_roots",
        &[
            (
                "package.json",
                "{\"name\": \"fs_roots\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_ROOTS),
            ("out/f.txt", "inner"),
            ("sub/g.txt", ""),
        ],
    );
    let sibling = |name: &str| {
        let dir = project.0.with_file_name(format!(
            "{}_{name}",
            project.0.file_name().unwrap().to_str().unwrap()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&dir);
        dir
    };
    let (r, w, l) = (sibling("r"), sibling("w"), sibling("l"));
    std::fs::create_dir_all(r.join("sub")).unwrap();
    std::fs::write(r.join("f.txt"), "read").unwrap();
    std::fs::write(r.join("sub/g.txt"), "deeper").unwrap();
    std::os::unix::fs::symlink("/etc", r.join("abs")).unwrap();
    std::fs::create_dir_all(w.join("sub")).unwrap();
    std::fs::write(w.join("f.txt"), "written").unwrap();
    std::os::unix::fs::symlink(&r, &l).unwrap();
    std::os::unix::fs::symlink(&r, project.0.join("to_r")).unwrap();
    std::os::unix::fs::symlink("..", project.0.join("out/up")).unwrap();
    let text = |path: &Path| path.display().to_string();
    let (r, w, l, p) = (text(&r), text(&w), text(&l), text(&project.0));
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-read={l}"),
        &format!("--allow-write={w},out"),
        "main.js",
        &r,
        &w,
        &l,
    ]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let written = (
        std::fs::read_to_string(Path::new(&w).join("x.txt")).ok(),
        Path::new(&w).join("sub/x").is_dir(),
        project.0.join("out/f.txt").exists(),
    );
    for dir in [&r, &w] {
        let _ = std::fs::remove_dir_all(dir);
    }
    let _ = std::fs::remove_file(&l);
    assert_eq!(written, (Some("x".to_owned()), true, false));
    let ended: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| panic!("{stderr}"));
    assert_eq!(
        ended,
        serde_json::json!({
            "read_by_alias": ["ok", "read"],
            "alias_is_real": ["ok", true],
            "real_path_by_alias": ["ok", true],
            "read_by_real": ["ok", "deeper"],
            "read_root_writes": ["FsError", "not_granted"],
            "write_only_reads": ["FsError", "not_granted"],
            "write_only_stats": ["FsError", "not_granted"],
            "write_only_lists": ["FsError", "not_granted"],
            "write_only_writes": ["ok", null],
            "write_only_sub_writes": ["ok", null],
            "write_only_sub_reads": ["FsError", "not_granted"],
            "write_only_readonly": ["FsError", "not_granted"],
            "write_only_real_path": ["FsError", "not_granted"],
            "inner_write_reads": ["ok", "inner"],
            "inner_write_writes": ["ok", null],
            "inner_write_climbs": ["ok", true],
            "project_dir_writes": ["FsError", "not_granted"],
            "project_writes": ["FsError", "not_granted"],
            "leaves_its_root": ["FsError", "not_granted"],
            "absolute_link_in_project": ["FsError", "not_granted"],
        })
    );
    assert_eq!(
        stderr,
        format!(
            "note: --allow-read={l} grants {r}\n\
             error: fs refused a write: not_granted: {r}/x.txt\n\
             error: fs refused a read: not_granted: {w}/f.txt\n\
             error: fs refused a read: not_granted: {w}\n\
             error: fs refused a read: not_granted: {w}/sub/g.txt\n\
             error: fs refused what the run grants: not_granted: {p}/out/x.txt\n\
             error: fs refused a write: not_granted: {p}/x.txt\n\
             error: fs refused a read: not_granted: {r}/abs\n\
             error: fs refused a read: not_granted: {p}/to_r\n\
             error: a refusal fails the run, whether or not the program caught it\n\
             {HINT_READ}\n\
             {HINT_WRITE}\n\
             {HINT_OPEN_DIR}\n"
        )
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A directory the flags list is resolved against the working directory
/// and must be one when the run starts, under `-A` too: a missing one, a
/// file, and a path through a file are usage errors naming it, escaped,
/// before anything runs.
#[test]
fn fs_flag_directories_are_checked_as_the_run_starts() {
    let project = Scratch::new(
        "fs_flag_dirs",
        &[
            (
                "package.json",
                "{\"name\": \"fs_flag_dirs\", \"type\": \"module\"}\n",
            ),
            ("main.js", "console.log('ran');\n"),
            ("file.txt", ""),
            ("dir/x.txt", ""),
        ],
    );
    let p = project.0.display().to_string();
    let help = "Run fgro run --help for more information.";
    for (args, message) in [
        (
            &["--allow-read=missing"][..],
            format!("a --allow-read directory doesn't exist: {p}/missing"),
        ),
        (
            &["--allow-write=dir,file.txt"],
            format!("a --allow-write directory isn't a directory: {p}/file.txt"),
        ),
        (
            &["--allow-read=file.txt/x"],
            format!("a --allow-read directory isn't a directory: {p}/file.txt/x"),
        ),
        (
            &["-A", "--allow-write=/nonexistent\u{1b}[2J"],
            "a --allow-write directory doesn't exist: /nonexistent\\u{1b}[2J".to_owned(),
        ),
        (
            &["--allow-read=dir", "--allow-write=gone"],
            format!("a --allow-write directory doesn't exist: {p}/gone"),
        ),
    ] {
        let args = [&["run"][..], args, &["main.js"]].concat();
        let mut command = fgro_command(&args);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("error: {message}\n{help}\n"),
            "{args:?}"
        );
        assert_eq!(output.stdout, b"", "{args:?}");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
    for args in [&["--allow-read=dir,.,/"][..], &["-A", "--allow-write=dir"]] {
        let args = [&["run"][..], args, &["main.js"]].concat();
        let mut command = fgro_command(&args);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(output.stdout, b"ran\n", "{args:?}");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
    }
}

/// Gives a directory its search permission back when dropped, a panic's
/// unwinding included, so `Scratch`'s drop can remove it; it never panics,
/// since a panic while unwinding aborts.
struct Unlock(PathBuf);

impl Drop for Unlock {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// A directory a flag lists that can't be looked at, beneath a directory
/// with no search permission, is a usage error naming it with the OS's
/// reason, under `-A` too, before anything runs.
#[test]
fn a_flag_directory_that_cant_be_looked_at_is_a_usage_error() {
    let project = Scratch::new(
        "fs_flag_unreadable",
        &[
            (
                "package.json",
                "{\"name\": \"fs_flag_unreadable\", \"type\": \"module\"}\n",
            ),
            ("main.js", "console.log('ran');\n"),
            ("locked/inner/x.txt", ""),
        ],
    );
    let p = project.0.display().to_string();
    project.set_mode("locked", 0o000);
    let unlock = Unlock(project.0.join("locked"));
    // a user the mode doesn't bind, root, looks anyway
    let looked = std::fs::metadata(project.0.join("locked/inner")).is_ok();
    let outputs = (!looked).then(|| {
        [
            &["--allow-read=locked/inner"][..],
            &["-A", "--allow-write=locked/inner"],
        ]
        .map(|args| {
            let args = [&["run"][..], args, &["main.js"]].concat();
            let mut command = fgro_command(&args);
            command.env_clear().current_dir(&project.0);
            (args.join(" "), spawn(command, b""))
        })
    });
    // restored before any assertion, so the directory is removed on drop
    drop(unlock);
    let Some(outputs) = outputs else {
        eprintln!(
            "skipped: {p}/locked/inner is looked at with its parent's mode 0o000 (running as root?)"
        );
        return;
    };
    let help = "Run fgro run --help for more information.";
    for ((args, output), flag) in outputs.into_iter().zip(["--allow-read", "--allow-write"]) {
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "error: a {flag} directory can't be looked at: {p}/locked/inner \
                 (Permission denied (os error 13))\n{help}\n"
            ),
            "{args}"
        );
        assert_eq!(output.stdout, b"", "{args}");
        assert_eq!(output.status.code(), Some(2), "{args}");
    }
}

/// A directory a flag lists whose real path isn't its text, resolved
/// against the working directory with its `.` and `..` by their text, is
/// noted with its real path as the run starts, before the program runs:
/// through a symlink, and by a `..` after one; never for one real already,
/// however it's spelled, and never under `-A`, whose directories grant
/// nothing more.
#[test]
fn a_flag_directory_through_a_symlink_is_noted_as_the_run_starts() {
    let project = Scratch::new(
        "fs_flag_notes",
        &[
            (
                "package.json",
                "{\"name\": \"fs_flag_notes\", \"type\": \"module\"}\n",
            ),
            ("main.js", "console.error('ran');\n"),
            ("data/f.txt", ""),
            ("sub/inner/f.txt", ""),
        ],
    );
    std::os::unix::fs::symlink("data", project.0.join("ln")).unwrap();
    std::os::unix::fs::symlink("sub/inner", project.0.join("deep_ln")).unwrap();
    let real = project.0.canonicalize().unwrap();
    let real = real.display();
    let run = |args: &[&str]| {
        let mut command = fgro_command(&[&["run"][..], args, &["main.js"]].concat());
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        String::from_utf8(output.stderr).unwrap()
    };
    assert_eq!(
        run(&[
            &format!("--allow-read=data,./data/,sub/../data,{real}/data,ln,deep_ln/.."),
            "--allow-write=ln,sub",
        ]),
        format!(
            "note: --allow-read=deep_ln/.. grants {real}/sub\n\
             note: --allow-read=ln grants {real}/data\n\
             note: --allow-write=ln grants {real}/data\n\
             ran\n"
        )
    );
    assert_eq!(
        run(&["-A", "--allow-read=ln", "--allow-write=deep_ln"]),
        "ran\n"
    );
}

/// The script [`fs_flag_roots_open_at_the_first_import`] runs: once the
/// test has changed the granted directories, the first import of `fs` and
/// a reach through each, as a JSON line.
const FS_LATE_ROOTS: &str = "import { args } from '@fuzdev/gro/args.ts';
const [r, w, u, l] = args;
console.log('ready');
for (let waited = 0; ; waited++) {
\tif (waited > 3000) throw Error('the test never changed the roots');
\ttry {
\t\tawait import('./go.js');
\t\tbreak;
\t} catch {}
\tawait new Promise((resolve) => setTimeout(resolve, 10));
}
const { fs } = await import('@fuzdev/gro/fs.ts');
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message, error.cause?.message ?? null];
\t}
};
console.log(JSON.stringify({
\treplaced: await how(async () => (await fs.open_dir(r)).read_text('f.txt')),
\tgone: await how(() => fs.open_dir(w)),
\tunchanged: await how(async () => (await fs.open_dir(u)).read_text('f.txt')),
\trepointed: await how(async () => (await fs.open_dir(l)).path),
}));
";

/// A granted directory is checked as the run starts and opened when the
/// run first imports `fs`, held to the directory it named at the start:
/// one replaced between, like one gone by then, fails every reach through
/// it with `io_error`, its cause saying which, which doesn't fail the run,
/// and one left alone is read. A written symlink pointed since at a
/// directory granted only read still names the directory it named, never
/// joining that root, which would read and write it.
#[test]
fn fs_flag_roots_open_at_the_first_import() {
    use std::io::BufRead as _;
    let project = Scratch::new(
        "fs_late_roots",
        &[
            (
                "package.json",
                "{\"name\": \"fs_late_roots\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_LATE_ROOTS),
            ("r/f.txt", "before"),
            ("w/f.txt", ""),
            ("u/f.txt", "kept"),
            ("v/f.txt", "v"),
        ],
    );
    let l = project.0.join("l");
    std::os::unix::fs::symlink("v", &l).unwrap();
    let (r, w, u) = (
        project.0.join("r"),
        project.0.join("w"),
        project.0.join("u"),
    );
    let text = |path: &Path| path.display().to_string();
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-read={},{}", text(&r), text(&u)),
        &format!("--allow-write={},{}", text(&w), text(&l)),
        "main.js",
        &text(&r),
        &text(&w),
        &text(&u),
        &text(&l),
    ]);
    command
        .env_clear()
        .current_dir(&project.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    std::fs::rename(&r, project.0.join("r_before")).unwrap();
    std::fs::create_dir(&r).unwrap();
    std::fs::write(r.join("f.txt"), "after").unwrap();
    std::fs::remove_dir_all(&w).unwrap();
    std::fs::remove_file(&l).unwrap();
    std::os::unix::fs::symlink("u", &l).unwrap();
    std::fs::write(project.0.join("go.js"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "note: --allow-write={} grants {}\n",
            text(&l),
            text(&project.0.join("v"))
        ),
        "{line}"
    );
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "replaced": [
                "FsError",
                "io_error",
                format!("the directory granted as {} was replaced since the run started", text(&r)),
            ],
            "gone": [
                "FsError",
                "io_error",
                format!("the directory granted as {} is gone, or no longer a directory", text(&w)),
            ],
            "unchanged": ["ok", "kept"],
            "repointed": ["ok", text(&project.0.join("v"))],
        })
    );
}

/// The script [`fs_a_flag_root_nests_as_the_run_started`] runs: once the
/// test has moved the written directory beneath the read one, the first
/// import of `fs` and a handle on the written one, then, once the test has
/// moved it back out, a read through that handle, as a JSON line.
const FS_MOVED_IN: &str = "import { args } from '@fuzdev/gro/args.ts';
const [wl] = args;
const wait = async (name) => {
\tfor (let waited = 0; ; waited++) {
\t\tif (waited > 3000) throw Error('the test never made ' + name);
\t\ttry {
\t\t\tawait import(name);
\t\t\treturn;
\t\t} catch {}
\t\tawait new Promise((resolve) => setTimeout(resolve, 10));
\t}
};
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message, error.cause?.message ?? null];
\t}
};
console.log('ready');
await wait('./moved_in.js');
const { fs } = await import('@fuzdev/gro/fs.ts');
let held = null;
const opened = await how(async () => {
\theld = await fs.open_dir(wl);
\treturn held.path;
});
console.log('opened');
await wait('./moved_out.js');
console.log(JSON.stringify({
\topened,
\theld: held && (await how(() => held.read_text('f.txt'))),
}));
";

/// A granted directory's root nests as its directory did when the run
/// started: a written directory moved beneath a read one before the first
/// import, its symlink pointed along, is gone from its real path, so no
/// handle on it reads it, there or once it's moved back out.
#[test]
fn fs_a_flag_root_nests_as_the_run_started() {
    use std::io::BufRead as _;
    let project = Scratch::new(
        "fs_moved_in",
        &[
            (
                "p/package.json",
                "{\"name\": \"fs_moved_in\", \"type\": \"module\"}\n",
            ),
            ("p/main.js", FS_MOVED_IN),
            ("r/kept.txt", ""),
            ("fake/w/f.txt", "written only"),
        ],
    );
    let at = |name: &str| project.0.join(name);
    let text = |path: &Path| path.display().to_string();
    let point = |target: &str| {
        std::os::unix::fs::symlink(target, at("wl.new")).unwrap();
        std::fs::rename(at("wl.new"), at("wl")).unwrap();
    };
    point("fake/w");
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-read={}", text(&at("r"))),
        &format!("--allow-write={}", text(&at("wl"))),
        "main.js",
        &text(&at("wl")),
    ]);
    command
        .env_clear()
        .current_dir(at("p"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    std::fs::rename(at("fake/w"), at("r/w")).unwrap();
    point("r/w");
    std::fs::write(at("p/moved_in.js"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "opened\n");
    std::fs::rename(at("r/w"), at("fake/w")).unwrap();
    point("fake/w");
    std::fs::write(at("p/moved_out.js"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "note: --allow-write={} grants {}\n",
            text(&at("wl")),
            text(&at("fake/w"))
        ),
        "{line}"
    );
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "opened": [
                "FsError",
                "io_error",
                format!(
                    "the directory granted as {}, {}, is gone, or no longer a directory",
                    text(&at("wl")),
                    text(&at("fake/w"))
                ),
            ],
            "held": null,
        })
    );
}

/// The script [`fs_unconfined_reads_and_writes_everything`] runs.
const FS_UNCONFINED: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const p = fs.project.path;
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message];
\t}
};
console.log(JSON.stringify({
\troot: await how(async () => (await fs.open_dir('/')).path),
\tthrough_absolute_link: await how(async () => (await fs.real_path(`${p}/to_outside/f.txt`)).endsWith('_outside/f.txt')),
\tclimbs_from_the_top: await how(async () => (await fs.open_dir(`/../..${p}`)).path === p),
\tproject_writes: await how(() => fs.project.write('x.txt', 'x')),
\treadonly_writes: await how(() => Object.getPrototypeOf(fs.project).write.call(fs.project.readonly(), 'x', 'y')),
\thandle_still_beneath: await how(() => fs.project.stat('to_outside/f.txt')),
}));
";

/// `-A` is one root at `/`, read and written, which resolves an absolute
/// path as the kernel does, an absolute symlink and a `..` at `/`
/// included, and holds the project's root, so `fs.project` writes too; a
/// handle still follows only links that stay beneath it, and nothing is
/// refused for a grant.
#[test]
fn fs_unconfined_reads_and_writes_everything() {
    let project = Scratch::new(
        "fs_unconfined",
        &[
            (
                "package.json",
                "{\"name\": \"fs_unconfined\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_UNCONFINED),
        ],
    );
    let outside = project.0.with_file_name(format!(
        "{}_outside",
        project.0.file_name().unwrap().to_str().unwrap()
    ));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("f.txt"), "").unwrap();
    std::os::unix::fs::symlink(&outside, project.0.join("to_outside")).unwrap();
    let mut command = fgro_command(&["run", "-A", "main.js"]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    let _ = std::fs::remove_dir_all(&outside);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "", "{stdout}");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "root": ["ok", "/"],
            "through_absolute_link": ["ok", true],
            "climbs_from_the_top": ["ok", true],
            "project_writes": ["ok", null],
            "readonly_writes": ["TypeError", "fs's methods are called on an fs handle of their kind, not another object"],
            "handle_still_beneath": ["FsError", "outside_root"],
        })
    );
}

/// The script [`fs_a_root_s_directory_is_neither_moved_nor_removed`] runs:
/// each remove and rename the rule refuses or allows, through a handle on
/// the directory given, as a JSON line.
const FS_RULE: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const [o_path] = args;
const o = await fs.open_dir(o_path);
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message];
\t}
};
console.log(JSON.stringify({
\tremove_a_root: await how(() => o.remove('r')),
\tremove_a_root_recursive: await how(() => o.remove('r', { recursive: true })),
\tremove_an_empty_root: await how(() => o.remove('e')),
\tremove_an_empty_root_slashed: await how(() => o.remove('e/')),
\tremove_a_holder_recursive: await how(() => o.remove('w', { recursive: true })),
\tremove_a_holder_through_a_handle: await how(() => o.dir('w').remove('inner', { recursive: true })),
\tremove_a_root_by_an_alias: await how(() => o.remove('lo/r', { recursive: true })),
\tremove_a_root_through_a_link: await how(() => o.remove('to_w/inner', { recursive: true })),
\trename_a_root_through_a_link: await how(() => o.rename('to_w/inner', 'inner')),
\trename_a_root: await how(() => o.rename('r', 'r2')),
\tremove_a_root_slashed: await how(() => o.remove('r/', { recursive: true })),
\trename_a_root_slashed: await how(() => o.rename('r/', 'r3')),
\trename_a_holder: await how(() => o.rename('w', 'w2')),
\trename_by_an_alias: await how(() => o.rename('lo/w', 'w2')),
\trename_over_an_empty_root: await how(() => o.rename('d', 'e')),
\trename_over_an_empty_root_slashed: await how(() => o.rename('d/', 'e/')),
\trename_into_a_root_it_reads: await how(() => o.rename('secret.txt', 'r/secret.txt')),
\trename_into_a_root_by_an_alias: await how(() => o.rename('lo/secret.txt', 'lo/r/secret.txt')),
\trename_a_directory_into_a_root_it_reads: await how(() => o.rename('d', 'r/d')),
\trename_into_a_root_it_writes: await how(() => o.rename('secret.txt', 'w/inner/secret.txt')),
\trename_out_of_a_root_it_reads: await how(() => o.rename('r/f.txt', 'f.txt')),
\trename_beside: await how(() => o.rename('w/a.txt', 'w/b.txt')),
\tremove_beside: await how(() => o.remove('w/b.txt')),
\tremove_inside_a_root: await how(() => o.remove('w/inner/keep.txt')),
}));
";

/// A root's directory, and any directory holding one, is neither removed
/// nor renamed through a handle that writes around it, whatever the run
/// grants: `io_error`, before anything is removed, for the root itself, an
/// empty one among them, a directory holding one, recursively or through a
/// handle on a directory beneath, by a path through a symlink to the
/// directory itself or to its parent, and a
/// rename over an empty root's directory, which would replace it, each
/// with a trailing `/` too. A rename
/// into a root that reads what the source's doesn't is refused too, a
/// directory's as a file's, and one into a root that grants nothing more,
/// or out of one, isn't; an entry beside a root or inside one moves and
/// goes. Nothing the rule refuses is a grant's refusal: none is listed,
/// and the run ends `0`.
#[test]
fn fs_a_root_s_directory_is_neither_moved_nor_removed() {
    let project = Scratch::new(
        "fs_rule",
        &[
            (
                "package.json",
                "{\"name\": \"fs_rule\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_RULE),
        ],
    );
    let o = Scratch::new(
        "fs_rule_o",
        &[
            ("secret.txt", "secret"),
            ("r/f.txt", "read"),
            ("w/a.txt", "a"),
            ("w/inner/keep.txt", "keep"),
        ],
    );
    for dir in ["e", "d"] {
        std::fs::create_dir(o.path(dir)).unwrap();
    }
    std::os::unix::fs::symlink(".", o.path("lo")).unwrap();
    std::os::unix::fs::symlink("w", o.path("to_w")).unwrap();
    let root = o.0.display().to_string();
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-write={root},{},{}", o.path("e"), o.path("w/inner")),
        &format!("--allow-read={}", o.path("r")),
        "main.js",
        &root,
    ]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "", "{stdout}");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&stdout).unwrap();
    let refused = serde_json::json!(["FsError", "io_error"]);
    let ok = serde_json::json!(["ok", null]);
    assert_eq!(
        ended,
        serde_json::json!({
            "remove_a_root": refused,
            "remove_a_root_recursive": refused,
            "remove_an_empty_root": refused,
            "remove_an_empty_root_slashed": refused,
            "remove_a_holder_recursive": refused,
            "remove_a_holder_through_a_handle": refused,
            "remove_a_root_by_an_alias": refused,
            "remove_a_root_through_a_link": refused,
            "rename_a_root_through_a_link": refused,
            "rename_a_root": refused,
            "remove_a_root_slashed": refused,
            "rename_a_root_slashed": refused,
            "rename_a_holder": refused,
            "rename_by_an_alias": refused,
            "rename_over_an_empty_root": refused,
            "rename_over_an_empty_root_slashed": refused,
            "rename_into_a_root_it_reads": refused,
            "rename_into_a_root_by_an_alias": refused,
            "rename_a_directory_into_a_root_it_reads": refused,
            "rename_into_a_root_it_writes": ok,
            "rename_out_of_a_root_it_reads": ok,
            "rename_beside": ok,
            "remove_beside": ok,
            "remove_inside_a_root": ok,
        })
    );
    let kept = |name: &str| Path::new(&o.path(name)).exists();
    assert!(kept("r") && kept("e") && kept("d") && kept("w/inner"));
    assert!(kept("w/inner/secret.txt") && kept("f.txt"));
    assert!(!kept("r/secret.txt") && !kept("r/d") && !kept("w/b.txt"));
    assert!(
        !kept("w/inner/keep.txt") && !kept("r2") && !kept("r3") && !kept("w2") && !kept("inner")
    );
}

/// The script [`fs_a_root_s_directory_is_kept_under_a_run_granted_everything`]
/// runs: a remove and a rename of the directory holding the project's.
const FS_RULE_UNCONFINED: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const above = await fs.open_dir(`${fs.project.path}/..`);
const how = async (call) => {
\ttry {
\t\treturn ['ok', (await call()) ?? null];
\t} catch (error) {
\t\treturn [error.constructor.name, error.kind ?? error.message];
\t}
};
console.log(JSON.stringify({
\tremove: await how(() => above.remove('proj', { recursive: true })),
\trename: await how(() => above.rename('proj', 'moved')),
\tremove_beside: await how(() => above.remove('beside.txt')),
}));
";

/// Under `-A`, which grants every path, the rule still holds the
/// project's root: the directory holding it is neither removed nor
/// renamed, and an entry beside it is removed.
#[test]
fn fs_a_root_s_directory_is_kept_under_a_run_granted_everything() {
    let scratch = Scratch::new(
        "fs_rule_unconfined",
        &[
            (
                "proj/package.json",
                "{\"name\": \"fs_rule_unconfined\", \"type\": \"module\"}\n",
            ),
            ("proj/main.js", FS_RULE_UNCONFINED),
            ("beside.txt", ""),
        ],
    );
    let mut command = fgro_command(&["run", "-A", "main.js"]);
    command.env_clear().current_dir(scratch.path("proj"));
    let output = spawn(command, b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "", "{stdout}");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "remove": ["FsError", "io_error"],
            "rename": ["FsError", "io_error"],
            "remove_beside": ["ok", null],
        })
    );
    assert!(Path::new(&scratch.path("proj/main.js")).exists());
    assert!(!Path::new(&scratch.path("beside.txt")).exists());
}

/// The script [`fs_a_write_s_paths_are_judged_by_the_handle_s_scope`] runs:
/// the call it names, caught, and then, for one that settled, what it left.
const FS_WRITE_GATE: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const [w, name] = args;
const handle = name.startsWith('project') ? fs.project : await fs.open_dir(w);
const calls = {
\tread_refused: () => handle.copy('f.txt', 'g.txt'),
\tproject_write_refused: () => handle.copy('package.json', 'g.txt'),
\tproject_above: () => handle.copy('../x', 'y'),
\tproject_absolute: () => handle.copy('/etc/passwd', 'y'),
\tabove: () => handle.copy('../x', 'y'),
\tabsolute: () => handle.copy('/etc/passwd', 'y'),
\tproject_set_times: () => handle.set_times('package.json', { mtime_ms: 0 }),
\tproject_set_mode: () => handle.set_mode('package.json', 0o600),
\tproject_symlink: () => handle.symlink('l', 'package.json'),
\tset_times: () => handle.set_times('f.txt', { mtime_ms: 1000 }),
\tset_mode: () => handle.set_mode('f.txt', 0o600),
\tsymlink: () => handle.symlink('l', 'f.txt'),
};
try {
\tconsole.log('settled', await calls[name]());
} catch (error) {
\tconsole.log(error.constructor.name, error.kind ?? error.message);
}
";

/// A copy reads its `from`, so a handle that doesn't read is refused it
/// `not_granted` naming `from`, judged before the write, with the hint a
/// read needs; a handle that reads but doesn't write is refused naming
/// `to`. A `from` above the handle or absolute is `outside_root` before any
/// grant is judged, through a handle that reads and one that writes alike,
/// and no refusal is recorded. `set_times`, `set_mode`, and `symlink` each
/// need the handle's write: through one that only reads, each is refused
/// naming its path, with the hint a write needs, and through one that
/// writes, each settles and does what it says, a write-only handle's
/// included, since none reads.
#[test]
fn fs_a_write_s_paths_are_judged_by_the_handle_s_scope() {
    let project = Scratch::new(
        "fs_copy_gate",
        &[
            (
                "package.json",
                "{\"name\": \"fs_copy_gate\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_WRITE_GATE),
        ],
    );
    let w = Scratch::new("fs_copy_gate_w", &[("f.txt", "f")]);
    let w_path = w.0.display().to_string();
    let run = |name: &str| {
        let mut command = fgro_command(&[
            "run",
            &format!("--allow-write={w_path}"),
            "main.js",
            &w_path,
            name,
        ]);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let p = project.0.display();
    let refusal = |need: &str, path: String, hint: &str| {
        format!(
            "error: fs refused {need}: not_granted: {path}\n\
             error: a refusal fails the run, whether or not the program caught it\n\
             {hint}\n"
        )
    };
    assert_eq!(
        run("read_refused"),
        (
            Some(1),
            "FsError not_granted\n".to_owned(),
            refusal("a read", format!("{w_path}/f.txt"), HINT_READ)
        )
    );
    assert_eq!(
        run("project_write_refused"),
        (
            Some(1),
            "FsError not_granted\n".to_owned(),
            refusal("a write", format!("{p}/g.txt"), HINT_WRITE)
        )
    );
    for name in ["project_above", "project_absolute", "above", "absolute"] {
        assert_eq!(
            run(name),
            (Some(0), "FsError outside_root\n".to_owned(), String::new()),
            "{name}"
        );
    }
    for name in ["project_set_times", "project_set_mode", "project_symlink"] {
        let path = if name == "project_symlink" {
            "l"
        } else {
            "package.json"
        };
        assert_eq!(
            run(name),
            (
                Some(1),
                "FsError not_granted\n".to_owned(),
                refusal("a write", format!("{p}/{path}"), HINT_WRITE)
            ),
            "{name}"
        );
    }
    let f = w.0.join("f.txt");
    for (name, settled) in [
        ("set_times", "undefined"),
        ("set_mode", "undefined"),
        ("symlink", "created"),
    ] {
        assert_eq!(
            run(name),
            (Some(0), format!("settled {settled}\n"), String::new()),
            "{name}"
        );
    }
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = std::fs::metadata(&f).unwrap();
    assert_eq!(
        (metadata.mtime(), metadata.permissions().mode() & 0o777),
        (1, 0o600)
    );
    assert_eq!(
        std::fs::read_link(w.0.join("l")).unwrap(),
        Path::new("f.txt")
    );
    let modes = std::fs::metadata(project.0.join("package.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(modes & 0o777, 0o600);
    assert!(!project.0.join("l").exists());
}

/// The script [`fs_a_copy_into_a_root_that_reads_is_refused_its_read`]
/// runs: a copy through a handle on the directory it's given, caught.
const FS_COPY_INTO_A_ROOT: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const o = await fs.open_dir(args[0]);
try {
\tawait o.copy('secret.txt', 'r/secret.txt');
\tconsole.log('copied');
} catch (error) {
\tconsole.log(error.kind, error.path);
}
";

/// The case the rule's scope gain exists for, a write-only root's file put
/// into a root the run reads, through a copy: a handle on the write root
/// doesn't read, so the copy is refused its read of `from`, `not_granted`
/// with the hint a read needs, before anything is written, and the rule
/// has nothing left to refuse. A copy is a read and a write through one
/// handle, so it never gains a scope the handle couldn't by a `read` and a
/// `write`.
#[test]
fn fs_a_copy_into_a_root_that_reads_is_refused_its_read() {
    let project = Scratch::new(
        "fs_copy_rule",
        &[
            (
                "package.json",
                "{\"name\": \"fs_copy_rule\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_COPY_INTO_A_ROOT),
        ],
    );
    let o = Scratch::new(
        "fs_copy_rule_o",
        &[("secret.txt", "secret"), ("r/f.txt", "")],
    );
    let root = o.0.display().to_string();
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-write={root}"),
        &format!("--allow-read={}", o.path("r")),
        "main.js",
        &root,
    ]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    assert_eq!(
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        ),
        (
            Some(1),
            format!("not_granted {root}/secret.txt\n"),
            format!(
                "error: fs refused a read: not_granted: {root}/secret.txt\n\
                 error: a refusal fails the run, whether or not the program caught it\n\
                 {HINT_READ}\n"
            ),
        )
    );
    assert!(!Path::new(&o.path("r/secret.txt")).exists());
}

/// The script [`fs_a_copy_drops_the_setuid_setgid_and_sticky_bits`] runs:
/// a file copied, then a tree, through a handle on the directory it's
/// given.
const FS_COPY_MODES: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const o = await fs.open_dir(args[0]);
await o.copy('s', 'c');
await o.copy('d', 'd2', { recursive: true });
";

/// What a copy makes takes its source's permission bits less the setuid,
/// setgid, and sticky bits, a file's and, in a tree, each directory's and
/// file's, as no write makes them.
#[test]
fn fs_a_copy_drops_the_setuid_setgid_and_sticky_bits() {
    use std::os::unix::fs::PermissionsExt as _;

    let project = Scratch::new(
        "fs_copy_modes",
        &[
            (
                "package.json",
                "{\"name\": \"fs_copy_modes\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_COPY_MODES),
        ],
    );
    let o = Scratch::new("fs_copy_modes_o", &[("s", "s"), ("d/in/f", "f")]);
    let mode = |path: &str, mode: u32| {
        std::fs::set_permissions(o.0.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
    };
    mode("s", 0o4755);
    mode("d/in/f", 0o2750);
    mode("d/in", 0o2755);
    mode("d", 0o1777);
    let root = o.0.display().to_string();
    let mut command = fgro_command(&[
        "run",
        &format!("--allow-write={root}"),
        &format!("--allow-read={root}"),
        "main.js",
        &root,
    ]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let modes: Vec<u32> = ["c", "d2", "d2/in", "d2/in/f"]
        .iter()
        .map(|path| {
            std::fs::metadata(o.0.join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        })
        .collect();
    assert_eq!(modes, [0o755, 0o777, 0o755, 0o750]);
}

/// The script [`fs_a_refused_move_of_a_root_hints_no_grant`] runs: the call
/// it names through `fs.project`, which doesn't write, caught.
const FS_ROOT_HINT: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const calls = {
\tremove_root: () => fs.project.remove('out'),
\tremove_holder: () => fs.project.remove('a', { recursive: true }),
\trename_root: () => fs.project.rename('out', 'x'),
\trename_over_root: () => fs.project.rename('x', 'out'),
\tremove_inside: () => fs.project.remove('out/f.txt'),
\tremove_own: () => fs.project.remove(''),
\trename_own: () => fs.project.rename('out', '.'),
};
try {
\tawait calls[args[0]]();
} catch (error) {
\tconsole.log(error.kind);
}
";

/// A remove or a rename through a handle that doesn't write, of a path
/// that is a root's directory or holds one by its text, is refused
/// `not_granted` and fails the run with no hint, since the rule refuses it
/// whatever is granted; a path inside a root still hints the handle to
/// open. The handle's own directory at an end is refused `io_error` before
/// the grant is judged, whatever the handle may do, so it's no refusal of
/// a grant.
#[test]
fn fs_a_refused_move_of_a_root_hints_no_grant() {
    let project = Scratch::new(
        "fs_root_hint",
        &[
            (
                "package.json",
                "{\"name\": \"fs_root_hint\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_ROOT_HINT),
            ("out/f.txt", ""),
            ("a/b/f.txt", ""),
        ],
    );
    let p = project.0.display();
    let run = |name: &str| {
        let mut command = fgro_command(&["run", "--allow-write=out,a/b", "main.js", name]);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(1), "{name}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "not_granted\n");
        String::from_utf8(output.stderr).unwrap()
    };
    // the line names the need as the hint does, or that there's none
    let refused = |path: &str, hint: Option<&str>| {
        format!(
            "error: fs refused {}: not_granted: {p}/{path}\n\
             error: a refusal fails the run, whether or not the program caught it\n{}",
            if hint.is_some() {
                "what the run grants"
            } else {
                "what no grant allows"
            },
            hint.map_or_else(String::new, |hint| format!("{hint}\n"))
        )
    };
    assert_eq!(run("remove_root"), refused("out", None));
    assert_eq!(run("remove_holder"), refused("a", None));
    assert_eq!(run("rename_root"), refused("out", None));
    assert_eq!(run("rename_over_root"), refused("x", None));
    assert_eq!(
        run("remove_inside"),
        refused("out/f.txt", Some(HINT_OPEN_DIR))
    );
    for name in ["remove_own", "rename_own"] {
        let mut command = fgro_command(&["run", "--allow-write=out,a/b", "main.js", name]);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(
            (
                output.status.code(),
                String::from_utf8(output.stdout).unwrap(),
                String::from_utf8(output.stderr).unwrap()
            ),
            (Some(0), "io_error\n".to_owned(), String::new()),
            "{name}"
        );
    }
}

/// The hint after an fs refusal that a grant to read lets through.
const HINT_READ: &str = "hint: add --allow-read naming the directory to grant what fs refused";

/// The hint after an fs refusal that a grant to write lets through, once
/// the directory granted is opened.
const HINT_WRITE: &str = "hint: add --allow-write naming the directory to grant what fs refused, \
                          then open it with fs.open_dir and write through that handle";

/// The hint after an fs refusal through a handle whose scope lacks the op,
/// on a directory a root already grants it in.
const HINT_OPEN_DIR: &str = "hint: the run grants what fs refused: \
                             open the directory with fs.open_dir and use that handle";

/// The script [`fs_hints_name_the_flag_and_no_path`] runs: a reach outside
/// every root by each path given.
const FS_HINTS: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
for (const path of args) {
\ttry {
\t\tawait fs.open_dir(path);
\t} catch {}
}
";

/// The hint after a refused path names the flag alone, once however many
/// paths were refused, never a path: not a real directory's, which the
/// refusal names, nor one a shell would read as something else, nor a
/// symlink's, `/proc/self/root`'s, a file's, or a missing one's, which
/// would grant another directory than it names, or nothing.
#[test]
fn fs_hints_name_the_flag_and_no_path() {
    let project = Scratch::new(
        "fs_hints",
        &[
            (
                "package.json",
                "{\"name\": \"fs_hints\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_HINTS),
        ],
    );
    let outside = Scratch::new("fs_hints_outside", &[("such-dir_1.0/f.txt", "")]);
    std::os::unix::fs::symlink("such-dir_1.0", outside.0.join("link")).unwrap();
    let run = |paths: &[&str]| {
        let args = [&["run", "main.js"][..], paths].concat();
        let mut command = fgro_command(&args);
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(1), "{paths:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        stderr
            .lines()
            .filter(|line| line.starts_with("hint:"))
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    };
    let at = |name: &str| format!("{}/{name}", outside.0.display());
    let paths = [
        at("such-dir_1.0"),
        format!("{}/../such-dir_1.0", at("such-dir_1.0")),
        "/etc".to_owned(),
        "/a b".to_owned(),
        "/a$(touch pwned)".to_owned(),
        "/a\nhint: forged".to_owned(),
        "/a\u{202e}b".to_owned(),
        "/a,b".to_owned(),
        "/no/such-dir_1.0".to_owned(),
        "/proc/self/root".to_owned(),
        at("link"),
        at("such-dir_1.0/f.txt"),
    ];
    for path in &paths {
        assert_eq!(run(&[path]), [HINT_READ], "{path:?}");
    }
    let all: Vec<&str> = paths.iter().map(String::as_str).collect();
    assert_eq!(run(&all), [HINT_READ]);
    assert!(!project.0.join("pwned").exists());
}

/// The script [`fs_a_scope_a_root_grants_hints_the_handle_to_open`] runs:
/// a write through `fs.project` to a directory `--allow-write=` grants,
/// uncaught; or each call it names, caught: that write, a read through a
/// handle on the directory given, only written, of a directory
/// `--allow-read=` grants inside it, two copies through `fs.project`,
/// into the directory written and out of it, and two renames, into it and
/// out of it.
const FS_PROJECT_WRITE: &str = "import { fs } from '@fuzdev/gro/fs.ts';
import { args } from '@fuzdev/gro/args.ts';
const [w, ...names] = args;
const calls = {
\twrite: () => fs.project.write('dist/x', 'x'),
\tread: async () => (await fs.open_dir(w)).read_text('sub/g.txt'),
\tcopy_in: () => fs.project.copy('main.js', 'dist/y'),
\tcopy_out: () => fs.project.copy('dist/.keep', 'x'),
\trename_in: () => fs.project.rename('main.js', 'dist/z'),
\trename_out: () => fs.project.rename('dist/.keep', 'z'),
};
if (names.length === 0) await fs.project.write('dist/x', 'x');
for (const name of names) {
\ttry {
\t\tawait calls[name]();
\t} catch (error) {
\t\tconsole.log(error.kind);
\t}
}
";

/// An op through a handle whose scope lacks it, on a directory a root
/// already grants, is refused, since a handle's scope never widens, and
/// its hint names no grant, which would widen the handle's whole root (the
/// project, for a write through `fs.project`), but a handle to open with
/// `fs.open_dir`, which does the op: a write into a directory written, a
/// copy's into it, judged by the path it writes, and a read through a root
/// only written of a directory read inside it. A copy out of the directory
/// written into one no root writes, judged by the path it writes too,
/// needs a directory granted and opened, and so does a rename into the
/// directory written or out of it, judged by both paths it writes and
/// named by its first; a read through a root only written, with no root reading what it
/// reads, needs a grant to read. Each line is printed once.
#[test]
fn fs_a_scope_a_root_grants_hints_the_handle_to_open() {
    let project = Scratch::new(
        "fs_project_write",
        &[
            (
                "package.json",
                "{\"name\": \"fs_project_write\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_PROJECT_WRITE),
            ("dist/.keep", ""),
        ],
    );
    let written = Scratch::new("fs_project_write_w", &[("sub/g.txt", "")]);
    let (read, write, open_dir) = ("a read", "a write", "what the run grants");
    let p = project.0.display();
    let w_sub = written.path("sub");
    let w = written.0.display().to_string();
    let run = |args: &[&str]| {
        let mut command = fgro_command(&[&["run"][..], args].concat());
        command.env_clear().current_dir(&project.0);
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    // each refusal by the kind of grant it needed, as its hint names it
    let refused = |paths: &[(&str, String)], hints: &[&str]| {
        let mut stderr = String::new();
        for (need, path) in paths {
            stderr.push_str("error: fs refused ");
            stderr.push_str(need);
            stderr.push_str(": not_granted: ");
            stderr.push_str(path);
            stderr.push('\n');
        }
        stderr.push_str("error: a refusal fails the run, whether or not the program caught it\n");
        for hint in hints {
            stderr.push_str(hint);
            stderr.push('\n');
        }
        ("not_granted\n".repeat(paths.len()), stderr)
    };
    assert_eq!(
        run(&["--allow-write=dist", "main.js"]),
        (
            String::new(),
            format!(
                "error: Uncaught (in promise) FsError: not_granted: {p}/dist/x\n    \
                 at file://{p}/main.js:12:42\n{HINT_OPEN_DIR}\n"
            )
        )
    );
    assert_eq!(
        run(&["--allow-write=dist", "main.js", "-", "copy_in"]),
        refused(&[(open_dir, format!("{p}/dist/y"))], &[HINT_OPEN_DIR])
    );
    assert_eq!(
        run(&["--allow-write=dist", "main.js", "-", "copy_out"]),
        refused(&[(write, format!("{p}/x"))], &[HINT_WRITE])
    );
    assert_eq!(
        run(&["--allow-write=dist", "main.js", "-", "rename_in"]),
        refused(&[(write, format!("{p}/main.js"))], &[HINT_WRITE])
    );
    assert_eq!(
        run(&["--allow-write=dist", "main.js", "-", "rename_out"]),
        refused(&[(write, format!("{p}/dist/.keep"))], &[HINT_WRITE])
    );
    assert_eq!(
        run(&[
            &format!("--allow-write={w}"),
            &format!("--allow-read={w_sub}"),
            "main.js",
            &w,
            "read",
        ]),
        refused(&[(open_dir, format!("{w}/sub/g.txt"))], &[HINT_OPEN_DIR])
    );
    assert_eq!(
        run(&[&format!("--allow-write={w}"), "main.js", &w, "read"]),
        refused(&[(read, format!("{w}/sub/g.txt"))], &[HINT_READ])
    );
    assert_eq!(
        run(&[
            &format!("--allow-write=dist,{w}"),
            &format!("--allow-read={w_sub}"),
            "main.js",
            &w,
            "write",
            "read",
            "copy_in",
            "copy_out",
        ]),
        refused(
            &[
                (open_dir, format!("{p}/dist/x")),
                (open_dir, format!("{w}/sub/g.txt")),
                (open_dir, format!("{p}/dist/y")),
                (write, format!("{p}/x")),
            ],
            &[HINT_WRITE, HINT_OPEN_DIR]
        )
    );
}

/// The script [`fs_handle_refuses_a_symlink_swapped_onto_its_path`] runs: a
/// handle on `sub`, made before the test swaps `sub` for a symlink, then
/// each read through it, as a JSON line.
const FS_SWAPPED: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const sub = fs.project.dir('sub');
console.log('ready');
for (let waited = 0; !fs.project.exists_sync('go'); waited++) {
\tif (waited > 3000) throw Error('the test never swapped sub');
\tawait new Promise((resolve) => setTimeout(resolve, 10));
}
const how = async (call) => {
\ttry {
\t\treturn ['fulfilled', await call()];
\t} catch (error) {
\t\treturn ['rejected', error.constructor.name, error.kind ?? null];
\t}
};
console.log(JSON.stringify({
\tstat: await how(() => sub.stat('in_other.txt').then((info) => info && info.kind)),
\texists: await how(() => sub.exists('in_sub.txt')),
\tdir: await how(async () => sub.dir('.').path),
\tthrough_the_root: await how(() => fs.project.stat('sub/in_other.txt').then((info) => info.kind)),
}));
";

/// A handle holds its directory's real path, so a symlink swapped onto that
/// path since it was made is refused as `outside_root`, never followed,
/// even one to a directory inside the project; a path through the root
/// still follows the same symlink, which stays inside.
#[test]
fn fs_handle_refuses_a_symlink_swapped_onto_its_path() {
    use std::io::BufRead as _;
    let project = Scratch::new(
        "fs_swapped",
        &[
            (
                "package.json",
                "{\"name\": \"fs_swapped\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_SWAPPED),
            ("sub/in_sub.txt", ""),
            ("other/in_other.txt", ""),
        ],
    );
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .env_clear()
        .current_dir(&project.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    std::fs::rename(project.0.join("sub"), project.0.join("sub_moved")).unwrap();
    std::os::unix::fs::symlink("other", project.0.join("sub")).unwrap();
    std::fs::write(project.0.join("go"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "stat": ["rejected", "FsError", "outside_root"],
            "exists": ["rejected", "FsError", "outside_root"],
            "dir": ["rejected", "FsError", "outside_root"],
            "through_the_root": ["fulfilled", "file"],
        })
    );
}

/// The script [`fs_real_path_refuses_a_root_moved_since_it_opened`] runs:
/// the root opened, then, once the test has moved the project away, each
/// reach by the project's old path, as a JSON line.
const FS_MOVED_ROOT: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const p = fs.project.path;
console.log('ready');
for (let waited = 0; !fs.project.exists_sync('go'); waited++) {
\tif (waited > 3000) throw Error('the test never moved the project');
\tawait new Promise((resolve) => setTimeout(resolve, 10));
}
const how = async (call) => {
\ttry {
\t\treturn ['fulfilled', await call()];
\t} catch (error) {
\t\treturn ['rejected', error.kind ?? null, String(error.cause?.message).includes('moved_away')];
\t}
};
console.log(JSON.stringify({
\treal_path: await how(() => fs.real_path(`${p}/sub/a.txt`)),
\twalked: await how(() => fs.real_path(`${p}/sub/../sub/a.txt`)),
\topen_dir: await how(async () => (await fs.open_dir(`${p}/sub`)).path),
}));
";

/// A root's descriptor follows its directory, so once the project moves
/// away, an entry reached by the old path has a real path outside the
/// root's: `io_error`, its cause naming no path outside the root, never a
/// path made of the outside one.
#[test]
fn fs_real_path_refuses_a_root_moved_since_it_opened() {
    use std::io::BufRead as _;
    let scratch = Scratch::new(
        "fs_moved_root",
        &[
            (
                "proj/package.json",
                "{\"name\": \"fs_moved_root\", \"type\": \"module\"}\n",
            ),
            ("proj/main.js", FS_MOVED_ROOT),
            ("proj/sub/a.txt", ""),
        ],
    );
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .env_clear()
        .current_dir(scratch.0.join("proj"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    std::fs::rename(scratch.0.join("proj"), scratch.0.join("moved_away")).unwrap();
    std::fs::write(scratch.0.join("moved_away/go"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "real_path": ["rejected", "io_error", false],
            "walked": ["rejected", "io_error", false],
            "open_dir": ["rejected", "io_error", false],
        })
    );
}

/// The script [`fs_reads_refuse_a_fifo_at_once`] runs: each whole-file read
/// of a FIFO no one writes to and of one a writer holds with bytes in it,
/// their kinds as `stat` and a listing see them, as a JSON line.
const FS_FIFOS: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const how = async (call) => {
\ttry {
\t\tconst value = await call();
\t\treturn ['fulfilled', value instanceof Uint8Array ? Array.from(value) : value];
\t} catch (error) {
\t\treturn ['rejected', error.kind ?? null, String(error.cause?.message)];
\t}
};
const ended = {};
for (const name of ['fifo', 'held']) {
\tended[name] = {
\t\tread: await how(() => fs.project.read(name)),
\t\tread_text: await how(() => fs.project.read_text(name)),
\t\tread_if_exists: await how(() => fs.project.read_if_exists(name)),
\t\tread_text_if_exists: await how(() => fs.project.read_text_if_exists(name)),
\t\tread_text_sync: await how(async () => fs.project.read_text_sync(name)),
\t\tthrough_a_link: await how(() => fs.project.read(`to_${name}`)),
\t\tstat: await how(async () => (await fs.project.stat(name)).kind),
\t};
}
const listed = [];
for await (const entry of fs.project.list()) listed.push(entry);
ended.listed = listed;
console.log(JSON.stringify(ended));
";

/// No whole-file read waits on a FIFO or reads one: one with no writer,
/// which the kernel would read as an empty file, and one a writer holds with
/// bytes in it, whose read would take them and then fail with `EAGAIN`, are
/// each refused at once as no regular file, `io_error`, through a symlink
/// too; `stat` and a listing see each as `other`. Node's own reads wait on
/// either until a writer closes it, which no differential case can show.
#[test]
fn fs_reads_refuse_a_fifo_at_once() {
    let project = Scratch::new(
        "fs_fifos",
        &[
            (
                "package.json",
                "{\"name\": \"fs_fifos\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_FIFOS),
        ],
    );
    for name in ["fifo", "held"] {
        let made = std::process::Command::new("mkfifo")
            .arg(project.0.join(name))
            .status()
            .unwrap();
        assert!(made.success());
        std::os::unix::fs::symlink(name, project.0.join(format!("to_{name}"))).unwrap();
    }
    // a writer, and a reader, on `held`, which `O_RDWR` opens at once, with
    // bytes in it
    let mut held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(project.0.join("held"))
        .unwrap();
    std::io::Write::write_all(&mut held, b"bytes").unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command.env_clear().current_dir(&project.0);
    let started = std::time::Instant::now();
    let output = spawn(command, b"");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    drop(held);
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_slice(&output.stdout).unwrap();
    let refused = |name: &str| {
        let why = "not a regular file (a FIFO or a device), whose contents are neither read nor \
                   written, since a read or a write of one could wait or never end";
        serde_json::json!(["rejected", "io_error", why]).to_string() + name
    };
    for name in ["fifo", "held"] {
        for read in [
            "read",
            "read_text",
            "read_if_exists",
            "read_text_if_exists",
            "read_text_sync",
            "through_a_link",
        ] {
            assert_eq!(
                ended[name][read].to_string() + name,
                refused(name),
                "{name} {read}"
            );
        }
        assert_eq!(
            ended[name]["stat"],
            serde_json::json!(["fulfilled", "other"])
        );
    }
    let listed: Vec<(&str, &str)> = ended["listed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap(),
                entry["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("fifo", "other"),
            ("held", "other"),
            ("main.js", "file"),
            ("package.json", "file"),
            ("to_fifo", "symlink"),
            ("to_held", "symlink"),
        ]
    );
}

/// The script [`fs_reads_dropped_stay_within_a_data_cap`] runs: reads of a
/// 16 MiB file, each dropped, twice the bytes of the cap it runs under, the
/// total printed.
const FS_DROPPED_READS: &str = "import { fs } from '@fuzdev/gro/fs.ts';
let total = 0;
for (let i = 0; i < 128; i++) total += (await fs.project.read('large')).length;
console.log(total);
";

/// A read's bytes are V8's own allocation, so its collector paces itself by
/// them and frees the ones a program dropped: reads totaling 2 GiB, each
/// dropped, run under a 1 GiB data cap (`RLIMIT_DATA`, util-linux's
/// `prlimit`), more than half of which a run with the JIT reserves before
/// any read. Bytes Rust allocated and handed over would pile up, V8 not
/// knowing to collect them, and the read past the cap would end the
/// process.
#[test]
fn fs_reads_dropped_stay_within_a_data_cap() {
    let project = Scratch::new(
        "fs_dropped_reads",
        &[
            (
                "package.json",
                "{\"name\": \"fs_dropped_reads\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_DROPPED_READS),
        ],
    );
    std::fs::write(project.0.join("large"), vec![7u8; 16 << 20]).unwrap();
    let cap = 1u64 << 30;
    let mut prlimit = std::process::Command::new("prlimit");
    prlimit.arg(format!("--data={cap}:{cap}"));
    let mut command = support::wrapped_fgro_command(prlimit, &["run", "main.js"]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", 128u64 << 24)
    );
}

/// A file read of the size it reports is in memory once, read straight into
/// the buffer V8 allocated of that size, never copied: a read of a 256 MiB
/// file peaks at less than 128 MiB more than the file (a start takes about
/// 40 MiB), where a copy would hold it twice. The file is sparse, so it
/// costs no disk, and its zeros are copied into the buffer all the same.
/// The peak is the kernel's `ru_maxrss` from the reaping, which takes this
/// test process's own if it's larger, as it isn't.
#[test]
fn fs_a_read_of_its_size_is_in_memory_once() {
    let project = Scratch::new(
        "fs_read_once",
        &[
            (
                "package.json",
                "{\"name\": \"fs_read_once\", \"type\": \"module\"}\n",
            ),
            (
                "main.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 const bytes = await fs.project.read('large');\n\
                 console.log(bytes.length, bytes.buffer.byteLength);\n",
            ),
        ],
    );
    let size: u64 = 256 << 20;
    std::fs::File::create(project.0.join("large"))
        .unwrap()
        .set_len(size)
        .unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .env_clear()
        .current_dir(&project.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let stdout = support::drain(child.stdout.take().unwrap());
    let stderr = support::drain(child.stderr.take().unwrap());
    let reaped = gro_sys::wait_child(child).unwrap();
    let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
    assert_eq!(
        reaped.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        format!("{size} {size}\n")
    );
    let bound = (size + (128 << 20)) >> 10;
    assert!(
        reaped.max_rss_kib < bound,
        "peaked at {} KiB, past {bound} KiB",
        reaped.max_rss_kib
    );
}

/// The script [`fs_list_fails_on_a_name_that_is_not_utf8`] runs: a listing
/// of a directory holding names that aren't UTF-8, and a read of a symlink
/// whose target isn't, as a JSON line.
const FS_NOT_UTF8: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const how = async (call) => {
\ttry {
\t\treturn ['fulfilled', await call()];
\t} catch (error) {
\t\treturn ['rejected', error.kind ?? null, error.path, String(error.cause?.message)];
\t}
};
const listing = fs.project.list('bytes');
console.log(JSON.stringify({
\tfirst: await how(() => listing.next()),
\tafter: await how(() => listing.next()),
\tread_link: await how(() => fs.project.read_link('bytes_link')),
\tthe_rest: await how(async () => {
\t\tconst names = [];
\t\tfor await (const entry of fs.project.list('ok')) names.push(entry.name);
\t\treturn names;
\t}),
}));
";

/// A listing of a directory holding a name that isn't UTF-8 fails whole,
/// at its first `next()`, before any entry, with `io_error` naming the
/// directory, its cause the least such name by its bytes (`m\xe2\x82`,
/// below `z\xfe\x80` and `\xff`), escaped, as the
/// Node module's does, since no string names what the kernel would reach;
/// the listing is done after; a symlink whose target isn't UTF-8 reads as
/// `io_error` the same way. Git holds no such name, and a checkout of one
/// fails where names are UTF-8 only, so the test makes them.
#[test]
fn fs_list_fails_on_a_name_that_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt as _;
    let project = Scratch::new(
        "fs_not_utf8",
        &[
            (
                "package.json",
                "{\"name\": \"fs_not_utf8\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_NOT_UTF8),
            ("bytes/fine", ""),
            ("ok/b", ""),
            ("ok/a", ""),
        ],
    );
    for name in [
        &b"\xff"[..],
        b"z\xfe\x80",
        b"\xfe\x80",
        b"y\xff",
        b"\xc3",
        b"m\xe2\x82",
        b"\xf0\x9f",
        b"zz\xff",
    ] {
        let path = project
            .0
            .join("bytes")
            .join(std::ffi::OsStr::from_bytes(name));
        std::fs::write(path, "").unwrap();
    }
    std::os::unix::fs::symlink(
        std::ffi::OsStr::from_bytes(b"to\xff"),
        project.0.join("bytes_link"),
    )
    .unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command.env_clear().current_dir(&project.0);
    let output = spawn(command, b"");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    let ended: Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = project.0.to_str().unwrap();
    assert_eq!(
        ended,
        serde_json::json!({
            "first": [
                "rejected",
                "io_error",
                format!("{root}/bytes"),
                format!("{root}/bytes holds an entry named m\\xe2\\x82, which isn't UTF-8"),
            ],
            "after": ["fulfilled", {"done": true}],
            "read_link": [
                "rejected",
                "io_error",
                format!("{root}/bytes_link"),
                format!("{root}/bytes_link is a symlink to to\\xff, which isn't UTF-8"),
            ],
            "the_rest": ["fulfilled", ["a", "b"]],
        })
    );
}

/// The script [`fs_listings_hold_no_descriptor`] runs: once warmed, ready;
/// then, once the test has counted its descriptors, listings ended every
/// way and kept alive, so none is collected; then, once counted again, it
/// ends.
const FS_LISTINGS: &str = "import { fs } from '@fuzdev/gro/fs.ts';
const wait = async (name) => {
\tfor (let waited = 0; !fs.project.exists_sync(name); waited++) {
\t\tif (waited > 3000) throw Error(`the test never made ${name}`);
\t\tawait new Promise((resolve) => setTimeout(resolve, 10));
\t}
};
for await (const entry of fs.project.list('dir')) void entry;
await new Promise((resolve) => setTimeout(resolve, 1));
console.log('ready');
await wait('go');
const kept = [];
for (let i = 0; i < 500; i++) {
\tconst abandoned = fs.project.list('dir');
\tawait abandoned.next();
\tconst returned = fs.project.list('dir');
\tawait returned.next();
\tawait returned.return();
\tconst thrown = fs.project.list('dir');
\tawait thrown.next();
\tawait thrown.throw(Error('thrown in')).catch(() => {});
\tconst broken = fs.project.list('dir');
\tfor await (const entry of broken) {
\t\tvoid entry;
\t\tbreak;
\t}
\tconst failed = fs.project.list('missing');
\tawait failed.next().catch(() => {});
\tkept.push(abandoned, returned, thrown, broken, failed, fs.project.list('dir'));
}
console.log(`listed ${kept.length}`);
await wait('done');
";

/// A listing holds no descriptor: its first `next()` reads the whole
/// directory, its descriptor closed before the call settles, so listings
/// abandoned after an entry, returned from, thrown into, broken out of,
/// failed, and never started, each kept alive so nothing is collected,
/// leave the process's descriptors as they were.
#[test]
fn fs_listings_hold_no_descriptor() {
    use std::io::BufRead as _;
    let project = Scratch::new(
        "fs_listings",
        &[
            (
                "package.json",
                "{\"name\": \"fs_listings\", \"type\": \"module\"}\n",
            ),
            ("main.js", FS_LISTINGS),
            ("dir/a", ""),
            ("dir/b", ""),
        ],
    );
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .env_clear()
        .current_dir(&project.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let descriptors = |pid: u32| {
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .unwrap()
            .count()
    };
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    let before = descriptors(child.id());
    std::fs::write(project.0.join("go"), "").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "listed 3000\n");
    let after = descriptors(child.id());
    std::fs::write(project.0.join("done"), "").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(after, before, "descriptors before and after the listings");
}
