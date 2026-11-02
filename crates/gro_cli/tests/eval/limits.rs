//! The caps: a module's bytes, the envelope's bytes, V8's heap, the process's
//! memory, the inherited stack limit, and the timeout.

use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::support::Scratch;
use crate::{eval, fixtures, long_timeout, memory_heavy_turn, parse, spawn, wrapped_command};

/// `size` bytes of a module whose first line is `first`, padded with a
/// comment.
fn padded_module(first: &str, size: usize) -> String {
    let mut source = format!("{first}\n//");
    source.push_str(&"x".repeat(size - source.len()));
    source
}

/// A module's source is capped at 16 MiB: one at the cap evaluates, and one
/// a byte over fails to load as `link`, naming the cap and the file, before
/// any of it is parsed or stripped (its syntax error, or its `enum`, isn't
/// reached), whether it's the entry or an import.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_module_past_its_byte_cap_fails_to_load() {
    // `gro_runtime::resolver::MAX_MODULE_SIZE`, which gro_runtime pins; a
    // literal keeps V8 out of this test binary
    let cap: usize = 16 << 20;
    let scratch = Scratch::new(
        "module_cap",
        &[
            ("package.json", r#"{"name": "p", "type": "module"}"#),
            ("importer.js", "export {default} from './over.js';\n"),
        ],
    );
    let write = |file: &str, first: &str, size: usize| {
        std::fs::write(scratch.0.join(file), padded_module(first, size)).unwrap();
    };
    write("at.js", "export default 'at the cap';", cap);
    write("over.js", "export default ) 'over';", cap + 1);
    write("over.ts", "enum E { A }", cap + 1);
    eval(&[&scratch.path("at.js")]).assert_value(&json!("at the cap"));
    for file in ["over.js", "over.ts"] {
        let run = eval(&[&scratch.path(file)]);
        run.assert_failed("link", 1);
        let url = format!("file://{}", scratch.path(file));
        assert!(
            run.message().contains(&format!(
                "cannot load {url}: the module is larger than 16 MiB, the cap on a module's \
                 source"
            )),
            "{}",
            run.message()
        );
        assert_eq!(run.envelope["error"]["specifier"], url.as_str());
    }
    let run = eval(&[&scratch.path("importer.js")]);
    run.assert_failed("link", 1);
    assert_eq!(
        run.envelope["error"]["specifier"],
        format!("file://{}", scratch.path("over.js")).as_str()
    );
    assert_eq!(
        run.envelope["error"]["importer"],
        format!("file://{}", scratch.path("importer.js")).as_str()
    );
}

/// The envelope is capped at 64 MiB: an envelope of exactly the cap is
/// written, one a byte over fails as `serialize`, naming the cap, and so
/// does a value whose JSON alone passes it, where the serializer stops
/// before reading the string.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn an_envelope_past_its_byte_cap_fails_as_serialize() {
    // strings of 64 MiB, and the envelope written around them
    let _turn = memory_heavy_turn();
    // `gro_runtime::eval::MAX_ENVELOPE_SIZE`, which gro_runtime pins; a
    // literal keeps V8 out of this test binary
    let cap: usize = 64 << 20;
    let scratch = Scratch::new(
        "envelope_cap",
        &[("package.json", r#"{"name": "p", "type": "module"}"#)],
    );
    let module = scratch.path("value.js");
    // the walk over a 64 MiB string is slow in a debug build
    let eval_repeat = |count: usize| {
        std::fs::write(&module, format!("export default 'x'.repeat({count});\n")).unwrap();
        eval(&[&module, "--timeout", &long_timeout()])
    };
    // the envelope around an empty string, so `count` x's make it that plus `count`
    let empty = eval_repeat(0);
    empty.assert_value(&json!(""));
    let around = empty.stdout.len() - 1;
    let at_cap = eval_repeat(cap - around);
    assert_eq!(at_cap.code, 0, "{}", at_cap.stderr);
    assert_eq!(at_cap.stdout.len() - 1, cap);
    assert_eq!(
        at_cap.envelope["value"].as_str().map(str::len),
        Some(cap - around)
    );
    let over = eval_repeat(cap - around + 1);
    over.assert_failed("serialize", 1);
    assert_eq!(
        over.message(),
        format!(
            "the envelope would be {} bytes, past 64 MiB, the envelope's cap, so the value isn't \
             written; write less data, and keep error messages and import specifiers short",
            cap + 1
        )
    );
    // the value's JSON alone past the cap: its quotes take it over
    let value_over = eval_repeat(cap - 1);
    value_over.assert_failed("serialize", 1);
    assert_eq!(
        value_over.message(),
        "$ takes the value's JSON past 64 MiB, the envelope's cap; write less data"
    );
    assert_eq!(value_over.envelope["error"]["at"], "$");
}

/// A failure's envelope is capped as a value's is, but keeps its kind: a
/// thrown error whose message alone passes 64 MiB fails as `module_threw`,
/// with no `access`, its message cut to 64 KiB and named by its length.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_failure_past_the_envelope_cap_keeps_its_kind() {
    // a message of 64 MiB, and its report copied out of the heap
    let _turn = memory_heavy_turn();
    let scratch = Scratch::new(
        "failure_cap",
        &[
            ("package.json", r#"{"name": "p", "type": "module"}"#),
            ("throw.js", "throw new Error('b'.repeat(64 << 20));\n"),
        ],
    );
    let run = eval(&[&scratch.path("throw.js"), "--timeout", &long_timeout()]);
    run.assert_failed("module_threw", 1);
    assert!(run.stdout.len() < 128 << 10, "{}", run.stdout.len());
    let message = run.message();
    let (kept, note) = message.split_once('…').unwrap();
    assert_eq!(kept.len(), 64 << 10);
    assert!(
        kept.starts_with("Uncaught (in promise) Error: bbb"),
        "{}",
        &kept[..64]
    );
    let length: usize = note
        .strip_prefix(" (")
        .and_then(|note| note.strip_suffix(" bytes)"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(length > 64 << 20, "{length}");
    assert_eq!(run.envelope["access"], Value::Null);
}

/// The message of a heap stop, and of the out-of-memory backstop's
/// envelope.
const HEAP_STOPPED: &str = "the eval's heap reached its cap of 512 MiB, and the run was stopped";
pub const HEAP_BACKSTOP: &str = "the eval's heap reached its cap of 512 MiB with nothing left to \
                             give: one operation V8 can't stop ran out of memory, so the process \
                             exited";

/// The heap's cap: growth the interpreter can stop is stopped as `heap`
/// with the run's envelope; under the JIT, which doesn't act on the stop,
/// and in one builtin call, which nothing stops, the out-of-memory backstop
/// writes its prepared `heap` envelope, with no `access`, and exits 1.
/// Run one after another, holding [`MEMORY_HEAVY`], so no two children hold
/// their heaps at once.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_heap_cap_fails_the_eval_as_heap() {
    let _turn = memory_heavy_turn();
    let run = eval(&["heap_grow.js"]);
    run.assert_failed("heap", 1);
    assert_eq!(run.message(), HEAP_STOPPED);
    assert_eq!(run.envelope["access"], json!({"dynamic": []}));
    for args in [
        &["heap_grow.js", "--jit"][..],
        &["heap_builtin.js"],
        &["heap_builtin.js", "--jit"],
    ] {
        let run = eval(args);
        run.assert_failed("heap", 1);
        assert_eq!(run.message(), HEAP_BACKSTOP, "{args:?}");
        assert_eq!(run.envelope["access"], Value::Null, "{args:?}");
        let id = fixtures().canonicalize().unwrap().join(args[0]);
        assert_eq!(
            run.envelope["entry"]["id"],
            id.to_str().unwrap(),
            "{args:?}"
        );
        assert_eq!(run.envelope["eval_protocol"], 2);
    }
}

/// How many times the string fixture runs. Its heap, read once the run has
/// stopped, reads all but empty in about a third of runs (jitless, a debug
/// build), so a stop read there, not in the callback, fails one of eight
/// runs about nineteen times in twenty.
const HEAP_STRINGS_RUNS: usize = 8;

/// A heap stop is `heap` by the heap the near-heap-limit callback reads,
/// at the moment V8 calls it: short strings grown to the cap, jitless, leave
/// a heap that, read once the run has stopped, is at times all but empty,
/// so each of [`HEAP_STRINGS_RUNS`] runs must still fail as `heap`; and a
/// map grown with the JIT runs short on one large table it grows into, the
/// callback reading the heap before the table is made, which the run's end
/// then reads in it. Either may end in the run's envelope or the
/// out-of-memory backstop's, both `heap`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_heap_stop_is_heap_however_the_heap_reads_after() {
    let _turn = memory_heavy_turn();
    let strings = std::iter::repeat_n(&["heap_strings.js"][..], HEAP_STRINGS_RUNS);
    for args in strings.chain([&["heap_map.js", "--jit"][..]]) {
        let run = eval(args);
        run.assert_failed("heap", 1);
        assert!(
            [HEAP_STOPPED, HEAP_BACKSTOP].contains(&run.message()),
            "{args:?}: {}",
            run.message()
        );
    }
}

/// The heap's cap is V8's heap only: an `ArrayBuffer` twice the cap is
/// made, since its bytes live outside the heap, within the process's memory
/// cap.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn array_buffer_memory_sits_outside_the_heap_cap() {
    let _turn = memory_heavy_turn();
    eval(&["heap_external.js"]).assert_value(&json!(1_073_741_824_u64));
}

/// The message of a run stopped by an allocation outside the heap, and of
/// the out-of-memory backstop's `memory` envelope.
const MEMORY_STOPPED: &str = "the eval ran out of memory outside the heap's cap: an allocation \
                              failed with the process at its memory cap of 2048 MiB, or a \
                              lower one its caller set, and the run was stopped";
const MEMORY_BACKSTOP: &str = "the eval ran out of memory outside the heap's cap: the process \
                               reached its memory cap of 2048 MiB, or a lower one its caller \
                               set, so it exited";

/// The process's memory cap, which `fgro eval` sets on itself: an
/// `ArrayBuffer` its size can't be made, and the failed allocation, after
/// which V8 runs the near-heap-limit callback as though the heap were full,
/// stops the run as `memory`, not `heap`, jitless or with the JIT. A lower
/// limit the caller set stands, and a heap that can't grow under it,
/// far below its own cap, runs V8 out of memory: the out-of-memory backstop
/// writes its prepared `memory` envelope, with no `access`, and exits 1.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_memory_cap_fails_the_eval_as_memory() {
    let _turn = memory_heavy_turn();
    for args in [
        &["memory_external.js"][..],
        &["memory_external.js", "--jit"],
    ] {
        let run = eval(args);
        run.assert_failed("memory", 1);
        assert_eq!(run.message(), MEMORY_STOPPED, "{args:?}");
        assert_eq!(run.envelope["access"], json!({"dynamic": []}), "{args:?}");
    }
    // the caller's lower limit, set by util-linux's `prlimit` before it
    // runs `fgro`: 256 MiB, under the heap's cap and above what the runtime
    // takes to start
    let limit = (256_u64 << 20).to_string();
    let mut prlimit = Command::new("prlimit");
    prlimit.arg(format!("--data={limit}:{limit}"));
    let run = parse(spawn(wrapped_command(prlimit, &["heap_grow.js"]), None));
    run.assert_failed("memory", 1);
    assert_eq!(run.message(), MEMORY_BACKSTOP);
    assert_eq!(run.envelope["access"], Value::Null);
}

/// A caller's large stack limit doesn't size the engine's threads: glibc
/// sizes a thread started without a size of its own (V8's platform
/// workers) by the `RLIMIT_STACK` the process inherited, each of those
/// stacks counted against the memory cap, so eval lowers the default first.
/// A 4 GiB stack limit, past the cap for one thread alone, set by
/// util-linux's `prlimit`, still gives the value, jitless or with the JIT.
/// An unlimited one, under which glibc takes a small default of its own
/// that eval keeps, is a smoke check that eval still starts.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_large_inherited_stack_limit_still_evaluates() {
    for stack in [(4_u64 << 30).to_string(), "unlimited".to_owned()] {
        for jit in [false, true] {
            let mut prlimit = Command::new("prlimit");
            prlimit.arg(format!("--stack={stack}:{stack}"));
            let args: &[&str] = if jit {
                &["value.js", "--jit"]
            } else {
                &["value.js"]
            };
            let run = parse(spawn(wrapped_command(prlimit, args), None));
            assert!(
                run.code == 0 && run.envelope["ok"] == true,
                "stack {stack}, jit {jit}: exit {}\n{}\n{}",
                run.code,
                run.stdout,
                run.stderr
            );
        }
    }
}

/// The timeout stops a loop that never ends, jitless or with the JIT, as
/// `timeout`, with a hint naming `--jit` only jitless.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_run_past_its_timeout_fails_as_timeout() {
    let run = eval(&["loop.js", "--timeout", "200"]);
    run.assert_failed("timeout", 1);
    assert_eq!(
        run.message(),
        "the eval ran past its timeout of 200 ms, and the run was stopped"
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval runs jitless unless asked, in V8's interpreter alone: --jit runs compute-heavy \
         code faster, or pass a longer --timeout"
    );
    let run = eval(&["loop.js", "--timeout", "200", "--jit"]);
    run.assert_failed("timeout", 1);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
}

/// The timeout stops the serializer's walk, which runs no JS for a
/// terminate to stop: this value's walk would outlast the test, and the stop
/// comes promptly, within a margin of the timeout that covers the process's
/// start and exit.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_timeout_stops_the_serializer_walk() {
    let timeout = Duration::from_millis(500);
    let started = Instant::now();
    let run = eval(&["walk.js", "--timeout", &timeout.as_millis().to_string()]);
    run.assert_failed("timeout", 1);
    let elapsed = started.elapsed();
    assert!(
        elapsed < timeout + Duration::from_secs(10),
        "the walk stopped {elapsed:?} after the eval started, past its {timeout:?} timeout"
    );
}
