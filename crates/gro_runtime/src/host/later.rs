//! Settling a native member's promise in a later turn: a member declared
//! `later` (`native.rs`) makes its promise and returns it pending, its work
//! goes on elsewhere (a thread, or a task on the event loop's own runtime),
//! and the work's result comes back to the isolate's thread, where the host
//! settles the promise as the event loop runs.
//!
//! The pieces are `deno_core`'s public API, so nothing here is an op, async
//! or not, nothing is in the snapshot, and no external reference is added.
//! A pending settlement is an entry in the run's table ([`LaterState`], in
//! `OpState`): its resolver, a `v8::Global<v8::PromiseResolver>`, under a
//! number. The work holds a [`Settler`], which is `Send` and holds no V8
//! value: the number, a `V8CrossThreadTaskSpawner` (the queue `deno_core`
//! drains on the isolate's thread each turn of the event loop, a microtask
//! checkpoint after), the `ExternalOpsTracker` the entry counts in, which
//! keeps the event loop from ending while the entry is pending, and the
//! member's `finish`, which makes the JS value of the work's `Send` result.
//! Settling posts a task that takes the entry out, uncounts it, makes the
//! value, and resolves or rejects; a task that finds no entry does nothing.
//! A post wakes the event loop from any thread at any moment, which
//! `deno_core`'s queue does only with gro's patch to it (`vendor/README.md`).
//!
//! The rules:
//!
//! - **Only the isolate's thread touches the table**, and the table holds
//!   the only V8 values of a settlement: a worker holds plain data, so a
//!   worker can't reach V8 however late it posts.
//! - **The count is the number of entries.** An entry is counted when it's
//!   made, on the call ([`defer`]), and uncounted when it's taken out, in
//!   the posted task, both on the isolate's thread, so whatever the threads
//!   do, nothing is uncounted unless an entry was removed: a second post
//!   for one entry, or one after the entry is gone, finds nothing and
//!   uncounts nothing.
//! - **A settlement always comes, unless its [`Settler`] is leaked.** A
//!   settler dropped unsettled (a worker that ended, a task its runtime
//!   dropped) posts a rejection, so a lost worker never holds the event
//!   loop open; one leaked (`mem::forget`, a cycle, a worker blocked for
//!   good) keeps its entry counted, and the event loop open, until the
//!   process ends. A task that finds the run's `OpState` borrowed mutably,
//!   which no host code leaves borrowed while JS can run, posts itself
//!   again rather than return with its entry still counted: `deno_core`
//!   runs what's posted up to three times in one poll, then wakes the event
//!   loop for another turn, so a borrow that outlasted a poll would spin
//!   the event loop, not leave it waiting.
//! - **No borrow is held while V8 runs a program's code.** Settling a value
//!   can read its `then`, which a program can define on a prototype, and
//!   that getter can call a native member, a later one included, so the
//!   entry is out of the table and every borrow of `OpState` and of the
//!   table dropped before the promise is resolved. A call makes its
//!   resolver, which can throw, before it inserts the entry, and hands its
//!   [`Deferred`] back only once the entry is counted, so a call ended at
//!   any step leaves the table and the count agreeing.
//! - **The promise is returned as made.** A later member's body returns a
//!   [`Deferred`], which only [`defer`] makes, so its callback returns the
//!   promise the host made, never one wrapped in another, which would read
//!   its `then`. A check or a body that throws before [`defer`] rejects the
//!   promise it returns, as an `async` member's does ([`settle_later`]).
//!
//! Eval never reaches any of it: eval's runtime holds no `Authority`, so a
//! member's check refuses before its body, the loader refuses every builtin
//! in an eval, and eval's runtime holds no [`LaterState`]. A settlement
//! left pending when a run ends goes with the process: `exit` ends every
//! thread, and a run that fails ends as it does with any work pending. An
//! embedder that drops a [`SpentRuntime`](crate::SpentRuntime) drops the
//! table's resolvers with the runtime's `OpState`, and the event loop's
//! runtime with the tasks it runs, whose settlers post into a queue nothing
//! drains, as does a worker that posts after: a post touches no V8 value,
//! and what's posted stays in the queue, which the spawner each post holds
//! keeps, until the process ends.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use deno_core::{ExternalOpsTracker, JsRuntime, OpState, V8CrossThreadTaskSpawner, v8};

use super::throw_error;

/// What a later member throws in a runtime started without the table of
/// pending settlements, which only a runtime made outside a run's start is.
const NO_LATER_STATE: &str = "this runtime was started without the host's pending settlements";

/// What a later member throws in a runtime whose `OpState` holds no
/// `V8CrossThreadTaskSpawner`, which `deno_core` puts in every runtime it
/// makes, so only a `deno_core` that stopped putting it there would throw it.
const NO_SPAWNER: &str = "this runtime holds no queue to settle a promise from in a later turn";

/// What a later member whose work is a task throws outside the event
/// loop's runtime, which only a test's runtime is.
const NO_EVENT_LOOP: &str = "a later member's task runs inside the event loop's runtime";

/// The rejection of a settlement whose work ended without a result.
pub const DROPPED: &str = "the host's work for this call ended without a result";

/// What makes the JS value a settlement's result is fulfilled with, on the
/// isolate's thread: `None` with an exception pending rejects with it, and
/// `None` with none fulfils with `undefined`.
pub type Finish<T> = for<'s> fn(&mut v8::PinScope<'s, '_>, T) -> Option<v8::Local<'s, v8::Value>>;

/// The run's pending settlements, put in its `OpState` when it starts,
/// touched only on the isolate's thread.
#[derive(Default)]
pub struct LaterState {
    /// The number the next entry takes.
    next: Cell<u64>,
    /// Each pending settlement's resolver, by its number.
    pending: RefCell<HashMap<u64, v8::Global<v8::PromiseResolver>>>,
}

impl LaterState {
    /// Inserts `resolver` under a number of its own, which it returns.
    fn insert(&self, resolver: v8::Global<v8::PromiseResolver>) -> u64 {
        let id = self.next.get();
        self.next.set(id.wrapping_add(1));
        self.pending.borrow_mut().insert(id, resolver);
        id
    }

    /// Takes entry `id` out, `None` when there is none.
    fn take(&self, id: u64) -> Option<v8::Global<v8::PromiseResolver>> {
        self.pending.borrow_mut().remove(&id)
    }

    /// How many settlements are pending.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.borrow().len()
    }
}

impl std::fmt::Debug for LaterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaterState")
            .field("pending", &self.pending.borrow().len())
            .finish_non_exhaustive()
    }
}

/// A promise a later member's body made with [`defer`], pending: the one
/// thing such a body returns, so its callback returns the promise as made.
#[derive(Debug)]
pub struct Deferred<'s>(v8::Local<'s, v8::Promise>);

impl<'s> Deferred<'s> {
    /// The promise, for a value a member makes beside the one it returns,
    /// held as made, as a started child's `closed` is.
    pub const fn promise(&self) -> v8::Local<'s, v8::Promise> {
        self.0
    }
}

/// What a later member's work holds to settle its promise: `Send`, and
/// holding no V8 value. Dropped unsettled, it rejects the promise with
/// [`DROPPED`].
pub struct Settler<T: Send + 'static> {
    /// Its settlement, until it's posted.
    delivery: Option<Delivery<T>>,
}

impl<T: Send + 'static> Settler<T> {
    /// Settles the promise with `value`, made into JS by the member's
    /// `finish` on the isolate's thread, in a later turn.
    pub fn settle(mut self, value: T) {
        if let Some(mut delivery) = self.delivery.take() {
            delivery.value = Some(value);
            delivery.post();
        }
    }
}

impl<T: Send + 'static> Drop for Settler<T> {
    fn drop(&mut self) {
        if let Some(delivery) = self.delivery.take() {
            delivery.post();
        }
    }
}

impl<T: Send + 'static> std::fmt::Debug for Settler<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settler").finish_non_exhaustive()
    }
}

/// A settlement on its way to the isolate's thread: its entry's number,
/// the queue it's posted to, the count its entry is in, how its value is
/// made, and the value, or none for one whose work ended without a result.
struct Delivery<T: Send + 'static> {
    id: u64,
    spawner: V8CrossThreadTaskSpawner,
    tracker: ExternalOpsTracker,
    finish: Finish<T>,
    value: Option<T>,
}

impl<T: Send + 'static> Delivery<T> {
    /// Posts it, to be delivered on the isolate's thread in a later turn,
    /// if the runtime is still there to run it. Touches no V8 value.
    fn post(self) {
        let spawner = self.spawner.clone();
        spawner.spawn(move |scope| deliver(scope, self));
    }
}

/// On the isolate's thread: takes `delivery`'s entry out, uncounts it, and
/// settles its promise with what `finish` makes of the value, or rejects it
/// with [`DROPPED`] for none. Nothing when the entry is gone; posted again
/// when the run's `OpState` is borrowed mutably, its entry still counted.
fn deliver<T: Send + 'static>(scope: &mut v8::PinScope<'_, '_>, delivery: Delivery<T>) {
    let state = JsRuntime::op_state_from(scope);
    let resolver = {
        let Ok(borrowed) = state.try_borrow() else {
            delivery.post();
            return;
        };
        borrowed
            .try_borrow::<LaterState>()
            .and_then(|later| later.take(delivery.id))
    };
    drop(state);
    let Some(resolver) = resolver else {
        return;
    };
    let Delivery {
        tracker,
        finish,
        value,
        ..
    } = delivery;
    // the entry is out: uncounted now, whatever settling it does
    tracker.unref_op();
    let resolver = v8::Local::new(scope, resolver);
    v8::tc_scope!(let tc, scope);
    let made = if let Some(value) = value {
        finish(tc, value)
    } else {
        throw_error(tc, DROPPED);
        None
    };
    if tc.has_terminated() {
        return;
    }
    let thrown = if tc.has_caught() {
        tc.exception()
    } else {
        None
    };
    tc.reset();
    match (made, thrown) {
        (_, Some(thrown)) => {
            resolver.reject(tc, thrown);
        }
        (Some(value), None) => {
            resolver.resolve(tc, value);
        }
        (None, None) => {
            let undefined = v8::undefined(tc).into();
            resolver.resolve(tc, undefined);
        }
    }
    tc.reset();
}

/// Makes a pending promise for a later member's call, counted so the event
/// loop waits for it, and hands `start` the [`Settler`] that settles it,
/// for the work to take wherever it runs: the body's last step. `None` with
/// an exception pending when the promise couldn't be made, and then nothing
/// was counted or started.
pub fn defer<'s, T: Send + 'static>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    finish: Finish<T>,
    start: impl FnOnce(Settler<T>),
) -> Option<Deferred<'s>> {
    let Some(later) = state.try_borrow::<LaterState>() else {
        throw_error(scope, NO_LATER_STATE);
        return None;
    };
    let Some(spawner) = state.try_borrow::<V8CrossThreadTaskSpawner>() else {
        throw_error(scope, NO_SPAWNER);
        return None;
    };
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let id = later.insert(v8::Global::new(scope, resolver));
    let tracker = state.external_ops_tracker.clone();
    tracker.ref_op();
    start(Settler {
        delivery: Some(Delivery {
            id,
            spawner: spawner.clone(),
            tracker,
            finish,
            value: None,
        }),
    });
    Some(Deferred(promise))
}

/// [`defer`] with the work a task on the event loop's own runtime, which
/// needs no thread: a timer, or a descriptor its reactor watches. `None`
/// with an `Error` thrown outside one.
pub fn defer_task<'s, T, F>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    finish: Finish<T>,
    work: impl FnOnce(Settler<T>) -> F,
) -> Option<Deferred<'s>>
where
    T: Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        throw_error(scope, NO_EVENT_LOOP);
        return None;
    };
    defer(state, scope, finish, |settler| {
        handle.spawn(work(settler));
    })
}

/// The promise a later member returns: the one its body made with
/// [`defer`], pending, or, when its check or its body threw before that,
/// one rejected with what it threw, as [`settle`](super::native::settle)
/// rejects. A call V8 terminated keeps its termination and returns no
/// promise.
pub fn settle_later<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    run: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Option<Deferred<'s>>,
) -> Option<v8::Local<'s, v8::Promise>> {
    v8::tc_scope!(let tc, scope);
    let deferred = run(tc);
    if tc.has_terminated() {
        tc.rethrow();
        return None;
    }
    if let Some(Deferred(promise)) = deferred {
        tc.reset();
        return Some(promise);
    }
    let thrown = if tc.has_caught() {
        tc.exception()
    } else {
        None
    };
    tc.reset();
    let resolver = v8::PromiseResolver::new(tc)?;
    if let Some(thrown) = thrown {
        resolver.reject(tc, thrown)?;
    } else {
        let undefined = v8::undefined(tc).into();
        resolver.resolve(tc, undefined)?;
    }
    Some(resolver.get_promise(tc))
}

#[cfg(test)]
mod tests;
