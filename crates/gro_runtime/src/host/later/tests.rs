//! The later form's tests, over members of their own table, declared
//! `later` through `native_members!` as a shipped member would be, on
//! runtimes from source that hold the run's table of pending settlements.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deno_core::{
    JsRuntime, OpState, PollEventLoopOptions, RuntimeOptions, V8CrossThreadTaskSpawner, v8,
};
use serde_json::{Value, json};

use super::{
    DROPPED, Deferred, LaterState, NO_LATER_STATE, NO_SPAWNER, Settler, defer, defer_task,
};
use crate::host::native::{self, Member, native_members};
use crate::host::{string, throw_error, throw_type_error};
use crate::runtime::SpentRuntime;
use crate::{ArgsGrant, Authority, Grants};

/// How long a drained event loop may take before a test calls it hung.
const HUNG: Duration = Duration::from_secs(10);

/// How many times `granted_wait`'s body ran, past its check.
static GRANTED_RAN: AtomicUsize = AtomicUsize::new(0);

/// Whether a `gated` worker may settle, and the condition it waits on.
static GATE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// The `gated` worker, to join.
static GATED: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// How many `gated` workers settled, each after its post returned.
static GATED_POSTED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// How many `task` futures this thread's event loop dropped, whether
    /// they settled or not: each test runs its event loop on its own
    /// thread, so another test's tasks never count here.
    static TASKS_DROPPED: Cell<usize> = const { Cell::new(0) };

    /// The barrier this thread's `herd` workers meet at, set for each
    /// round.
    static HERD: RefCell<Option<Arc<Barrier>>> = const { RefCell::new(None) };
}

/// What a test member's work ends with.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// Fulfils with the number.
    Fulfil(f64),
    /// Rejects with an `Error` naming the number.
    Reject(f64),
    /// Fulfils with a fresh `{value}` of the realm's `Object.prototype`,
    /// so settling it reads a `then` a program defined there.
    Object(f64),
}

/// The test members' `finish`.
fn finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    outcome: Outcome,
) -> Option<v8::Local<'s, v8::Value>> {
    match outcome {
        Outcome::Fulfil(value) => Some(v8::Number::new(scope, value).into()),
        Outcome::Reject(value) => {
            throw_error(scope, &format!("rejected {value}"));
            None
        }
        Outcome::Object(value) => {
            let object = v8::Object::new(scope);
            let key = string(scope, "value")?;
            let value = v8::Number::new(scope, value);
            object.create_data_property(scope, key.into(), value.into())?;
            Some(object.into())
        }
    }
}

/// Counts a `task` future's drop.
struct Counted;

impl Drop for Counted {
    fn drop(&mut self) {
        TASKS_DROPPED.set(TASKS_DROPPED.get() + 1);
    }
}

/// `wait(producer, outcome, ms)`: settles with `outcome` (`fulfil`,
/// `reject`, `object`, or `drop`, its settler dropped unsettled) from
/// `producer`: posted from the call itself (`now`), a task on the event
/// loop's runtime after `ms` (`task`), a thread of its own after `ms`
/// (`thread`), a thread that waits for [`GATE`] (`gated`), or a thread
/// that meets the rest of its [`HERD`] at the barrier and then spins for
/// `ms`, a fraction of a millisecond (`herd`).
fn wait<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    producer: v8::Local<'s, v8::Value>,
    outcome: v8::Local<'s, v8::Value>,
    ms: v8::Local<'s, v8::Value>,
) -> Option<Deferred<'s>> {
    let text = |value: v8::Local<'s, v8::Value>| {
        v8::Local::<v8::String>::try_from(value)
            .ok()
            .map(|text| text.to_rust_string_lossy(scope))
    };
    let (Some(producer), Some(outcome), Ok(ms)) = (
        text(producer),
        text(outcome),
        v8::Local::<v8::Number>::try_from(ms),
    ) else {
        throw_type_error(scope, "wait takes two strings and a number");
        return None;
    };
    let ms = ms.value();
    let settled = match outcome.as_str() {
        "fulfil" => Some(Outcome::Fulfil(ms)),
        "reject" => Some(Outcome::Reject(ms)),
        "object" => Some(Outcome::Object(ms)),
        "drop" => None,
        _ => {
            throw_type_error(scope, "no such outcome");
            return None;
        }
    };
    let delay = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
    let end = move |settler: Settler<Outcome>| match settled {
        Some(outcome) => settler.settle(outcome),
        None => drop(settler),
    };
    match producer.as_str() {
        "now" => defer(state, scope, finish, end),
        "task" => defer_task(state, scope, finish, move |settler| {
            let counted = Counted;
            async move {
                let _counted = counted;
                tokio::time::sleep(delay).await;
                end(settler);
            }
        }),
        "thread" => defer(state, scope, finish, |settler| {
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                end(settler);
            });
        }),
        "herd" => {
            let Some(herd) = HERD.with_borrow(Option::clone) else {
                throw_type_error(scope, "no herd to join");
                return None;
            };
            defer(state, scope, finish, |settler| {
                std::thread::spawn(move || {
                    herd.wait();
                    let started = Instant::now();
                    while started.elapsed() < delay {
                        std::hint::spin_loop();
                    }
                    end(settler);
                });
            })
        }
        "gated" => defer(state, scope, finish, |settler| {
            let worker = std::thread::spawn(move || {
                let (open, opened) = &GATE;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = opened.wait(open).unwrap();
                }
                drop(open);
                end(settler);
                GATED_POSTED.fetch_add(1, Ordering::Relaxed);
            });
            *GATED.lock().unwrap() = Some(worker);
        }),
        _ => {
            throw_type_error(scope, "no such producer");
            None
        }
    }
}

native_members! {
    /// `wait(producer, outcome, ms)` ([`wait`]).
    later wait(state; scope, producer, outcome, ms) for "@fuzdev/gro/blake3.ts" {
        wait(state, scope, producer, outcome, ms)
    }

    /// `granted_wait()`: fulfils with 1, from the call, once the run's
    /// `args` grant is checked; counts its body's runs in
    /// [`GRANTED_RAN`].
    later granted_wait(state, _granted: ArgsGrant; scope) for "@fuzdev/gro/args.ts" {
        GRANTED_RAN.fetch_add(1, Ordering::Relaxed);
        defer(state, scope, finish, |settler| settler.settle(Outcome::Fulfil(1.0)))
    }
}

/// The JS the tests define first: `results`, each settlement's by its
/// label, and `record`, which records one there.
const HELPERS: &str = "globalThis.results = {};
globalThis.record = (label, promise) => promise.then(
	(value) => { results[label] = ['fulfilled', typeof value === 'object' ? { ...value } : value]; },
	(error) => { results[label] = ['rejected', `${error.constructor.name}: ${error.message}`]; },
);";

/// Makes a runtime from source over a fresh event loop's runtime, entered,
/// holding `authority` (none for `None`) and an empty table of pending
/// settlements, with the test table's members as globals and [`HELPERS`]
/// run, and hands both to `test`.
fn on_runtime<R>(
    authority: Option<Authority>,
    test: impl FnOnce(&mut JsRuntime, &tokio::runtime::Runtime) -> R,
) -> R {
    let event_loop = event_loop();
    let _entered = event_loop.enter();
    let mut runtime = new_runtime(authority);
    test(&mut runtime, &event_loop)
}

/// A current-thread event loop's runtime, as a run's is.
fn event_loop() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// [`on_runtime`]'s runtime, made inside the event loop's runtime.
fn new_runtime(authority: Option<Authority>) -> JsRuntime {
    let mut runtime = JsRuntime::new(RuntimeOptions::default());
    let state = runtime.op_state();
    if let Some(authority) = authority {
        state.borrow_mut().put(authority);
    }
    state.borrow_mut().put(LaterState::default());
    drop(state);
    {
        deno_core::scope!(scope, &mut runtime);
        let global = scope.get_current_context().global(scope);
        for member in NATIVE_MEMBERS {
            let function = native::probe::function_of(scope, member).unwrap();
            let key = v8::String::new(scope, member.name).unwrap();
            global.set(scope, key.into(), function.into()).unwrap();
        }
    }
    runtime.execute_script("<test>", HELPERS).unwrap();
    runtime
}

/// A run that grants `args` alone.
fn granting_args() -> Authority {
    Authority::Confined(Grants {
        args: Some(ArgsGrant {}),
        ..Grants::default()
    })
}

/// Runs `source`.
fn run(runtime: &mut JsRuntime, source: &str) {
    runtime.execute_script("<test>", source.to_owned()).unwrap();
}

/// The settlements recorded so far.
fn results(runtime: &mut JsRuntime) -> Value {
    let value = runtime
        .execute_script("<test>", "JSON.stringify(results)")
        .unwrap();
    deno_core::scope!(scope, runtime);
    let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
    serde_json::from_str(&json).unwrap()
}

/// How many settlements the run's table holds.
fn pending(runtime: &JsRuntime) -> usize {
    runtime.op_state().borrow().borrow::<LaterState>().len()
}

/// Runs the event loop until nothing is pending: `false` when it's still
/// pending after [`HUNG`], or failed.
fn drain(runtime: &mut JsRuntime, event_loop: &tokio::runtime::Runtime) -> bool {
    drain_within(runtime, event_loop, HUNG)
}

/// Runs the event loop until nothing is pending: `false` when it's still
/// pending after `limit`, or failed.
fn drain_within(
    runtime: &mut JsRuntime,
    event_loop: &tokio::runtime::Runtime,
    limit: Duration,
) -> bool {
    event_loop.block_on(async {
        tokio::time::timeout(
            limit,
            runtime.run_event_loop(PollEventLoopOptions::default()),
        )
        .await
        .is_ok_and(|ended| ended.is_ok())
    })
}

/// The test table's members are functions of the form: `async` to the
/// pins, of the `length` they declare.
#[test]
fn a_later_member_is_an_async_function() {
    for (member, declared) in NATIVE_MEMBERS.iter().zip([3, 0]) {
        let Member::Function {
            length,
            asynchronous,
            ..
        } = member.member
        else {
            panic!("{} is no function", member.name);
        };
        assert_eq!((length, asynchronous), (declared, true), "{}", member.name);
    }
}

/// Each producer fulfils, rejects, and, its settler dropped unsettled,
/// rejects as dropped, each in a later turn: nothing is settled when the
/// calls return, each call's entry is counted, so the event loop waits for
/// a thread's post, and once all are settled the table is empty and the
/// event loop ends, which it does only with the count at zero.
#[test]
fn a_later_member_settles_from_each_producer() {
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        let producers = ["now", "task", "thread"];
        let outcomes = ["fulfil", "reject", "drop"];
        let calls: String = producers
            .iter()
            .flat_map(|producer| {
                outcomes.iter().map(move |outcome| {
                    format!(
                        "record('{producer} {outcome}', wait('{producer}', '{outcome}', 20));\n"
                    )
                })
            })
            .collect();
        run(runtime, &calls);
        assert_eq!(results(runtime), json!({}));
        assert_eq!(pending(runtime), producers.len() * outcomes.len());
        assert!(drain(runtime, event_loop), "the event loop hung");
        let mut expected = serde_json::Map::new();
        for producer in producers {
            expected.insert(format!("{producer} fulfil"), json!(["fulfilled", 20]));
            expected.insert(
                format!("{producer} reject"),
                json!(["rejected", "Error: rejected 20"]),
            );
            expected.insert(
                format!("{producer} drop"),
                json!(["rejected", format!("Error: {DROPPED}")]),
            );
        }
        assert_eq!(results(runtime), Value::Object(expected));
        assert_eq!(pending(runtime), 0);
    });
}

/// A module's top-level `await` on a later member's promise waits for a
/// thread's post rather than ending as a stalled `await`: the count keeps
/// the module's evaluation pending as it keeps the event loop open.
#[test]
fn a_top_level_await_waits_for_a_later_settlement() {
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        let ended = event_loop.block_on(async {
            let specifier = deno_core::ModuleSpecifier::parse("file:///main.js").unwrap();
            let id = runtime
                .load_main_es_module_from_code(
                    &specifier,
                    "results.awaited = await wait('thread', 'fulfil', 20);",
                )
                .await
                .unwrap();
            let evaluated = runtime.mod_evaluate(id);
            tokio::time::timeout(HUNG, async {
                runtime
                    .run_event_loop(PollEventLoopOptions::default())
                    .await?;
                evaluated.await
            })
            .await
        });
        assert!(matches!(ended, Ok(Ok(()))), "{ended:?}");
        assert_eq!(results(runtime), json!({"awaited": 20}));
        assert_eq!(pending(runtime), 0);
    });
}

/// A settlement whose task finds the run's `OpState` borrowed mutably comes
/// again in a later turn, its entry counted until then, rather than
/// leaving its entry counted with nothing to settle it, and uncounting
/// nothing when it comes again: a thread's settlement pending beside it
/// still holds the event loop open.
#[test]
fn a_settlement_that_finds_the_state_busy_comes_again() {
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        run(
            runtime,
            "record('busy', wait('now', 'fulfil', 3)); record('beside', wait('thread', 'fulfil', 100));",
        );
        let state = runtime.op_state();
        let held = state.borrow_mut();
        let mut context = Context::from_waker(Waker::noop());
        let polled = runtime.poll_event_loop(&mut context, PollEventLoopOptions::default());
        drop(held);
        assert!(matches!(polled, Poll::Pending));
        assert_eq!(results(runtime), json!({}));
        assert_eq!(pending(runtime), 2);
        assert!(drain(runtime, event_loop), "the event loop hung");
        assert_eq!(
            results(runtime),
            json!({"busy": ["fulfilled", 3], "beside": ["fulfilled", 100]})
        );
        assert_eq!(pending(runtime), 0);
    });
}

/// A later member refuses without its grant before its body runs: the
/// promise it returns is rejected with the refusal, nothing is counted,
/// and the event loop ends; granted, it runs. One that throws before it
/// defers, for a bad argument, rejects with what it threw, counting
/// nothing.
#[test]
fn a_refused_later_member_defers_nothing() {
    // a grant's refusal reads the same with no authority at all
    let not_granted = "Error: not_granted: granted_wait of @fuzdev/gro/args.ts needs the grant \
                       'args', which this runtime doesn't hold";
    for (authority, expected) in [
        (Some(Authority::default()), json!(["rejected", not_granted])),
        (None, json!(["rejected", not_granted])),
        (Some(granting_args()), json!(["fulfilled", 1])),
    ] {
        let granted = expected[0] == "fulfilled";
        on_runtime(authority, |runtime, event_loop| {
            let ran = GRANTED_RAN.load(Ordering::Relaxed);
            run(
                runtime,
                "record('call', granted_wait()); record('bad', wait(1, 'fulfil', 1));",
            );
            assert_eq!(pending(runtime), usize::from(granted));
            assert_eq!(
                GRANTED_RAN.load(Ordering::Relaxed),
                ran + usize::from(granted)
            );
            assert!(drain(runtime, event_loop), "the event loop hung");
            let bad = if authority_holds(runtime) {
                json!(["rejected", "TypeError: wait takes two strings and a number"])
            } else {
                json!([
                    "rejected",
                    "Error: not_granted: wait of @fuzdev/gro/blake3.ts is a host \
                                    function, and this runtime holds no authority, so it runs \
                                    none"
                ])
            };
            assert_eq!(results(runtime), json!({"call": expected, "bad": bad}));
            assert_eq!(pending(runtime), 0);
        });
    }
}

/// Whether the runtime holds an authority.
fn authority_holds(runtime: &JsRuntime) -> bool {
    runtime.op_state().borrow().has::<Authority>()
}

/// A later member's call in a runtime that lacks what [`defer`] needs
/// rejects naming what's missing: the table of pending settlements, or,
/// with the table there, the queue a settlement is posted to. Neither
/// counts anything.
#[test]
fn a_later_member_names_what_its_runtime_lacks() {
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        let state = runtime.op_state();
        let later = state.borrow_mut().take::<LaterState>();
        run(runtime, "record('no table', wait('now', 'fulfil', 1));");
        state.borrow_mut().put(later);
        let spawner = state.borrow_mut().take::<V8CrossThreadTaskSpawner>();
        run(runtime, "record('no queue', wait('now', 'fulfil', 2));");
        state.borrow_mut().put(spawner);
        drop(state);
        assert_eq!(pending(runtime), 0);
        assert!(drain(runtime, event_loop), "the event loop hung");
        assert_eq!(
            results(runtime),
            json!({
                "no table": ["rejected", format!("Error: {NO_LATER_STATE}")],
                "no queue": ["rejected", format!("Error: {NO_SPAWNER}")],
            })
        );
    });
}

/// A settled object's `then`, which a program defined on
/// `Object.prototype`, is read once, as any promise's resolution reads it;
/// a getter that throws rejects the promise with what it threw; and one
/// that calls later members as the host settles finds the host free: each
/// of those settles, and the table empties.
#[test]
fn a_hostile_then_is_read_once_and_finds_the_host_free() {
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        run(
            runtime,
            "globalThis.reads = 0;
globalThis.mode = 'count';
Object.defineProperty(Object.prototype, 'then', {
	configurable: true,
	get() {
		if (!Object.hasOwn(this, 'value')) return undefined;
		reads++;
		if (mode === 'throw') throw new Error('then threw');
		if (mode === 'reenter') {
			mode = 'count';
			record('inner now', wait('now', 'fulfil', 4));
			record('inner thread', wait('thread', 'fulfil', 5));
			record('inner bad', wait(0, 'fulfil', 6));
		}
		return undefined;
	},
});
record('counted', wait('now', 'object', 1));",
        );
        assert!(drain(runtime, event_loop), "the event loop hung");
        run(
            runtime,
            "results.reads = reads; mode = 'throw'; record('thrown', wait('thread', 'object', 2));",
        );
        assert!(drain(runtime, event_loop), "the event loop hung");
        run(
            runtime,
            "mode = 'reenter'; record('reentered', wait('now', 'object', 3));",
        );
        assert!(drain(runtime, event_loop), "the event loop hung");
        assert_eq!(
            results(runtime),
            json!({
                "counted": ["fulfilled", {"value": 1}],
                "reads": 1,
                "thrown": ["rejected", "Error: then threw"],
                "reentered": ["fulfilled", {"value": 3}],
                "inner now": ["fulfilled", 4],
                "inner thread": ["fulfilled", 5],
                "inner bad": ["rejected", "TypeError: wait takes two strings and a number"],
            })
        );
        assert_eq!(pending(runtime), 0);
    });
}

/// An embedder that drops a spent runtime with settlements pending drops
/// them with it, and a worker that posts afterwards touches nothing: its
/// post goes into a queue nothing drains. A task's future goes with the
/// event loop's runtime, its settler posting its rejection the same way.
#[test]
fn a_spent_runtime_dropped_with_settlements_pending_takes_late_posts() {
    let event_loop = event_loop();
    let runtime = {
        let _entered = event_loop.enter();
        let mut runtime = new_runtime(Some(Authority::default()));
        run(
            &mut runtime,
            "record('gated', wait('gated', 'fulfil', 1));
record('task', wait('task', 'fulfil', 600000));
record('now', wait('now', 'object', 1));",
        );
        runtime
    };
    assert_eq!(pending(&runtime), 3);
    let tasks_dropped = TASKS_DROPPED.get();
    drop(SpentRuntime::new(runtime, event_loop));
    assert_eq!(TASKS_DROPPED.get(), tasks_dropped + 1);
    let (open, opened) = &GATE;
    *open.lock().unwrap() = true;
    opened.notify_all();
    let worker = GATED.lock().unwrap().take().unwrap();
    worker.join().unwrap();
    assert_eq!(GATED_POSTED.load(Ordering::Relaxed), 1);
}

/// Threads released together, each posting after a few microseconds' spin,
/// round after round: each round's posts wake the event loop, so the round
/// ends well inside `STALLED`. A post that lands between the task queue's
/// check for tasks and its register of the waker wakes nothing unless the
/// register comes first (`vendor/README.md`, the patch to `tasks.rs`), and
/// the round then sleeps until the timeout's own timer wakes it.
#[test]
#[ignore = "a stress test of the task queue's wakeup: \
            `cargo test -p gro_runtime --lib -- --ignored a_herd_of_threads`"]
fn a_herd_of_threads_posting_at_once_never_leaves_the_event_loop_asleep() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 10_000;
    const STALLED: Duration = Duration::from_secs(1);
    on_runtime(Some(Authority::default()), |runtime, event_loop| {
        let mut stalled = Vec::new();
        for round in 0..ROUNDS {
            HERD.set(Some(Arc::new(Barrier::new(THREADS))));
            run(
                runtime,
                &format!(
                    "results = {{}};
for (let i = 0; i < {THREADS}; i++) record(i, wait('herd', 'fulfil', (({round} * 7 + i * 13) % 50) / 1000));"
                ),
            );
            let started = Instant::now();
            let ended = drain_within(runtime, event_loop, STALLED);
            if started.elapsed() >= STALLED {
                stalled.push(round);
            }
            assert!(ended || drain(runtime, event_loop), "the event loop hung");
            assert_eq!(
                results(runtime).as_object().map(serde_json::Map::len),
                Some(THREADS)
            );
            assert_eq!(pending(runtime), 0);
        }
        assert!(stalled.is_empty(), "rounds that stalled: {stalled:?}");
    });
}

/// A waker that counts its wakes.
#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// Posts a chain of `left` tasks to `spawner`, each counting itself in
/// `ran` and posting the next.
fn chain(spawner: V8CrossThreadTaskSpawner, left: usize, ran: Arc<AtomicUsize>) {
    spawner.clone().spawn(move |_| {
        ran.fetch_add(1, Ordering::Relaxed);
        if left > 1 {
            chain(spawner, left - 1, ran);
        }
    });
}

/// A turn that runs every task posted to the queue leaves the turn's waker
/// registered with it, so a post after the turn wakes the event loop, however
/// many of its looks at the queue found a task: `deno_core` looks again after
/// each batch it runs, up to a limit, and a look that found a task registered
/// nothing until gro's patch to it (`vendor/README.md`, the patch to
/// `tasks.rs`). A chain of tasks, each posting the next, ends the turn on such
/// a look when its length is the limit; a longer one leaves a task queued and
/// the turn wakes itself.
#[test]
fn a_post_after_a_turn_that_ran_every_posted_task_wakes_the_event_loop() {
    let mut capped = false;
    for length in 1..=8 {
        on_runtime(Some(Authority::default()), |runtime, _| {
            let spawner = runtime
                .op_state()
                .borrow()
                .borrow::<V8CrossThreadTaskSpawner>()
                .clone();
            let ran = Arc::new(AtomicUsize::new(0));
            chain(spawner.clone(), length, Arc::clone(&ran));
            let wakes = Arc::new(Wakes::default());
            let waker = Waker::from(Arc::clone(&wakes));
            let _ = runtime.poll_event_loop(
                &mut Context::from_waker(&waker),
                PollEventLoopOptions::default(),
            );
            if ran.load(Ordering::Relaxed) < length {
                capped = true;
                return;
            }
            let before = wakes.0.load(Ordering::Relaxed);
            spawner.spawn(|_| {});
            assert!(
                wakes.0.load(Ordering::Relaxed) > before,
                "a post after a turn that ran a chain of {length} tasks woke nothing"
            );
        });
    }
    assert!(capped, "no chain outran one turn's looks at the queue");
}
