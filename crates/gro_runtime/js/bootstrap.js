// gro's bootstrap: installs the pure web globals deno_web provides, then
// removes the host's internals from the global object.
//
// Pure globals only: nothing here reaches the network, the filesystem, the
// environment, or another process. Authority globals (`fetch` and the like)
// will install only under a grant. The set is the part of the WinterTC
// minimum common API that deno_web carries (`fetch` and `crypto` live in other
// crates, not linked). `reportError`, `self`, and the global object as an
// `EventTarget` wait for a consumer: deno_web's `reportError` dispatches on a
// global event target this bootstrap doesn't set up.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { core } from 'ext:core/mod.js';
import { op_gro_color, op_gro_console_stderr, op_gro_deny } from 'ext:core/ops';

const load = core.loadExtScript;

const console_ns = load('ext:deno_web/01_console.js');
const url_ns = load('ext:deno_web/00_url.js');
const url_pattern_ns = load('ext:deno_web/01_urlpattern.js');
const dom_exception_ns = load('ext:deno_web/01_dom_exception.js');
const event_ns = load('ext:deno_web/02_event.js');
const structured_clone_ns = load('ext:deno_web/02_structured_clone.js');
const timers_ns = load('ext:deno_web/02_timers.js');
const abort_signal_ns = load('ext:deno_web/03_abort_signal.js');
const base64_ns = load('ext:deno_web/05_base64.js');
const streams_ns = load('ext:deno_web/06_streams.js');
const text_encoding_ns = load('ext:deno_web/08_text_encoding.js');
const file_ns = load('ext:deno_web/09_file.js');
const compression_ns = load('ext:deno_web/14_compression.js');
const performance_ns = load('ext:deno_web/15_performance.js');

// the run's console options, read from the host by `gro_start` when each run
// starts; the snapshot carries only these placeholders, never a run's answer
let all_to_stderr = false;
let color_stdout = false;
let color_stderr = false;

// deno_web asks the first for the levels that go to stdout by default and the
// second for the rest, so each answers for the stream its levels actually
// write to
console_ns.setNoColorFns(
	() => !(all_to_stderr ? color_stderr : color_stdout),
	() => !color_stderr
);

/** @type {Record<string, unknown>} */
const globals = {
	console: new console_ns.Console((/** @type {string} */ message, /** @type {number} */ level) =>
		// `warn`, `error`, and `trace` (levels 2 and up) go to stderr, and every
		// level does when the run says so
		core.print(message, all_to_stderr || level > 1)
	),

	URL: url_ns.URL,
	URLSearchParams: url_ns.URLSearchParams,
	URLPattern: url_pattern_ns.URLPattern,

	DOMException: dom_exception_ns.DOMException,
	Event: event_ns.Event,
	EventTarget: event_ns.EventTarget,
	CustomEvent: event_ns.CustomEvent,
	ErrorEvent: event_ns.ErrorEvent,
	PromiseRejectionEvent: event_ns.PromiseRejectionEvent,

	structuredClone: structured_clone_ns.structuredClone,

	setTimeout: timers_ns.setTimeout,
	clearTimeout: timers_ns.clearTimeout,
	setInterval: timers_ns.setInterval,
	clearInterval: timers_ns.clearInterval,

	AbortController: abort_signal_ns.AbortController,
	AbortSignal: abort_signal_ns.AbortSignal,

	atob: base64_ns.atob,
	btoa: base64_ns.btoa,

	ReadableStream: streams_ns.ReadableStream,
	ReadableStreamDefaultReader: streams_ns.ReadableStreamDefaultReader,
	ReadableStreamBYOBReader: streams_ns.ReadableStreamBYOBReader,
	ReadableStreamBYOBRequest: streams_ns.ReadableStreamBYOBRequest,
	ReadableByteStreamController: streams_ns.ReadableByteStreamController,
	ReadableStreamDefaultController: streams_ns.ReadableStreamDefaultController,
	TransformStream: streams_ns.TransformStream,
	TransformStreamDefaultController: streams_ns.TransformStreamDefaultController,
	WritableStream: streams_ns.WritableStream,
	WritableStreamDefaultWriter: streams_ns.WritableStreamDefaultWriter,
	WritableStreamDefaultController: streams_ns.WritableStreamDefaultController,
	ByteLengthQueuingStrategy: streams_ns.ByteLengthQueuingStrategy,
	CountQueuingStrategy: streams_ns.CountQueuingStrategy,

	TextEncoder: text_encoding_ns.TextEncoder,
	TextDecoder: text_encoding_ns.TextDecoder,
	TextEncoderStream: text_encoding_ns.TextEncoderStream,
	TextDecoderStream: text_encoding_ns.TextDecoderStream,

	Blob: file_ns.Blob,
	File: file_ns.File,

	CompressionStream: compression_ns.CompressionStream,
	DecompressionStream: compression_ns.DecompressionStream,

	performance: performance_ns.performance,

	// deno_core's own, redefined here so it has the shape of the rest whether
	// the runtime starts from a snapshot or not (the snapshot build leaves it
	// enumerable)
	queueMicrotask: /** @type {any} */ (globalThis).queueMicrotask
};

// the web's shape for interface objects and operations: writable,
// configurable, not enumerable
for (const [name, value] of Object.entries(globals)) {
	Object.defineProperty(globalThis, name, {
		value,
		writable: true,
		enumerable: false,
		configurable: true
	});
}

// the steps that must happen in each run, never in a snapshot: the
// runtime calls this once it has started, from a snapshot or not. deno_core
// reads `Deno.core` while it starts from a snapshot, so the internals stay
// until then.
/** @type {any} */ (globalThis).__bootstrap.gro_start = () => {
	performance_ns.setTimeOrigin();
	all_to_stderr = op_gro_console_stderr();
	color_stdout = op_gro_color(false);
	color_stderr = op_gro_color(true);
	if (op_gro_deny()) deny();
	// deno_core's internals: `Deno.core` reaches every op, and gro has no
	// `Deno` namespace. Hygiene, not a boundary: no grant check relies on this.
	delete (/** @type {any} */ (globalThis).__bootstrap);
	delete (/** @type {any} */ (globalThis).Deno);
};

// What `deny` says a refused operation would make depend on, after its name.
const WHY = {
	clock:
		'reads the clock, so the value would differ from run to run; write the date or time ' +
		"as a literal with its offset, such as new Date('2026-01-01T00:00:00Z')",
	random:
		'is random, so the value would differ from run to run; write the value as a literal, ' +
		"or derive it from the module's own data",
	machine: 'is a fact of this process or machine, so the value would differ from host to host',
	timer: 'schedules work on the clock, and eval has no timers; chain promise jobs instead'
};

/**
 * The error a refused operation throws: its name and the reason.
 * @param {string} shown
 * @param {keyof typeof WHY} why
 */
const refusal = (shown, why) => new Error(`${shown} is refused in eval (${why}): it ${WHY[why]}`);

/**
 * A function named `name` that throws the refusal whenever it's called.
 * @param {string} name
 * @param {string} shown
 * @param {keyof typeof WHY} why
 */
const refuser = (name, shown, why) =>
	/** @type {Record<string, () => never>} */ ({
		[name]() {
			throw refusal(shown, why);
		}
	})[name];

/**
 * Replaces `owner[name]` in place, keeping the descriptor's writability,
 * enumerability, and configurability: a data property's value, or an
 * accessor's getter, by what `replace` makes of the original. An owner
 * without the property is left alone.
 * @param {any} owner
 * @param {string} name
 * @param {(original: any) => any} replace
 */
const replace_member = (owner, name, replace) => {
	const descriptor = Object.getOwnPropertyDescriptor(owner, name);
	if (descriptor === undefined) return;
	if ('value' in descriptor) {
		descriptor.value = replace(descriptor.value);
	} else {
		descriptor.get = replace(descriptor.get);
	}
	Object.defineProperty(owner, name, descriptor);
};

/**
 * `determinism: deny`, eval's only mode: every operation that reads the
 * clock, randomness, or a fact of the machine, or schedules a timer,
 * throws a refusal naming it and the reason; the garbage collector is made
 * unobservable; and the timers, `Blob`, and `File` are absent. Runs once
 * per run, before any user or driver code, after the time origin is set.
 * The originals are replaced where they live, so no global or member user
 * code can name leads to one: they survive in this function's closures and
 * in the web classes' own module scopes, which no code in the run can read.
 *
 * `Blob` and `File` are absent rather than refused member by member: a
 * `File` keeps the time it was made in a symbol-keyed field (which
 * `Object.getOwnPropertySymbols` and `structuredClone` reach past the
 * `lastModified` getter), and each blob part keeps a random id, so neither
 * class can be made deterministic from outside. `URL.createObjectURL` and
 * `URL.revokeObjectURL` exist only for blobs and go with them.
 *
 * Owners the runtime lacks are skipped by construction: `crypto`,
 * `navigator`, the storage globals, `Worker`, and `PerformanceMark` aren't
 * installed, so they need no entry.
 */
const deny = () => {
	const global = /** @type {any} */ (globalThis);
	const performance_prototype = Object.getPrototypeOf(performance_ns.performance);

	// functions that throw whenever they're called
	/** @type {Array<[any, string, string, keyof typeof WHY]>} */
	const closed_functions = [
		[Math, 'random', 'Math.random()', 'random'],
		[performance_prototype, 'now', 'performance.now()', 'clock'],
		[performance_prototype, 'mark', 'performance.mark()', 'clock'],
		[performance_prototype, 'measure', 'performance.measure()', 'clock'],
		[performance_prototype, 'toJSON', 'performance.toJSON()', 'clock'],
		[abort_signal_ns.AbortSignal, 'timeout', 'AbortSignal.timeout()', 'timer']
	];
	for (const name of Object.getOwnPropertyNames(global.Temporal.Now)) {
		closed_functions.push([
			global.Temporal.Now,
			name,
			`Temporal.Now.${name}()`,
			name === 'timeZoneId' ? 'machine' : 'clock'
		]);
	}
	for (const [owner, name, shown, why] of closed_functions) {
		replace_member(owner, name, () => refuser(name, shown, why));
	}

	// getters that throw whenever they're read
	/** @type {Array<[any, string, string, keyof typeof WHY]>} */
	const closed_getters = [[performance_prototype, 'timeOrigin', 'performance.timeOrigin', 'clock']];
	for (const [owner, name, shown, why] of closed_getters) {
		replace_member(owner, name, () => refuser(`get ${name}`, shown, why));
	}

	// `Date`: no reading of the clock (`Date()`, `new Date()`, `Date.now()`);
	// `Date.UTC`, `Date.parse`, and `new Date(value)` are kept. The guard
	// stands where `Date` stood: it shares its prototype, whose `constructor`
	// names the guard.
	{
		const RealDate = Date;
		/**
		 * @this {unknown}
		 * @param {Array<any>} args
		 */
		const GuardedDate = function Date(...args) {
			if (new.target === undefined) throw refusal('Date()', 'clock');
			if (args.length === 0) throw refusal('new Date()', 'clock');
			return Reflect.construct(RealDate, args, new.target);
		};
		Object.defineProperties(GuardedDate, {
			length: { value: RealDate.length },
			prototype: { value: RealDate.prototype, writable: false }
		});
		replace_member(RealDate.prototype, 'constructor', () => GuardedDate);
		for (const name of ['UTC', 'parse']) {
			const descriptor = /** @type {PropertyDescriptor} */ (
				Object.getOwnPropertyDescriptor(RealDate, name)
			);
			Object.defineProperty(GuardedDate, name, descriptor);
		}
		const now = /** @type {PropertyDescriptor} */ (
			Object.getOwnPropertyDescriptor(RealDate, 'now')
		);
		now.value = refuser('now', 'Date.now()', 'clock');
		Object.defineProperty(GuardedDate, 'now', now);
		replace_member(global, 'Date', () => GuardedDate);
	}

	// `Intl.DateTimeFormat` formats the present when given no date
	{
		const prototype = Intl.DateTimeFormat.prototype;
		/**
		 * @param {string} shown
		 * @param {(...args: Array<any>) => any} real
		 */
		const dated = (shown, real) =>
			/** @this {any} */
			function (/** @type {Array<any>} */ ...args) {
				if (args[0] === undefined) throw refusal(`${shown} with no date`, 'clock');
				return Reflect.apply(real, this, args);
			};
		replace_member(prototype, 'format', (get) =>
			Object.defineProperty(
				/** @this {any} */
				function () {
					return dated('Intl.DateTimeFormat format()', get.call(this));
				},
				'name',
				{ value: 'get format' }
			)
		);
		replace_member(prototype, 'formatToParts', (real) =>
			dated('Intl.DateTimeFormat formatToParts()', real)
		);
	}

	// `Atomics.waitAsync` with a finite timeout settles on the clock; with
	// none, or one already elapsed (zero or less), it settles by its value
	// alone. The timeout is read once, as the original would read it.
	{
		const real = Atomics.waitAsync;
		replace_member(
			Atomics,
			'waitAsync',
			() =>
				/** @type {Record<string, (...args: Array<any>) => any>} */ ({
					waitAsync(/** @type {Array<any>} */ ...args) {
						if (args.length > 3) {
							const timeout = Number(args[3]);
							if (Number.isFinite(timeout) && timeout > 0) {
								throw refusal('Atomics.waitAsync() with a timeout', 'timer');
							}
							args[3] = timeout;
						}
						return Reflect.apply(real, Atomics, args);
					}
				}).waitAsync
		);
	}

	// the garbage collector, made unobservable rather than refused: a
	// `WeakRef` that holds its target, so `deref()` always returns it, and a
	// registry whose callback never runs
	{
		/** @param {unknown} target */
		const collectable = (target) => {
			const type = typeof target;
			if (target === null || (type !== 'object' && type !== 'function' && type !== 'symbol')) {
				throw new TypeError('invalid value used as a weak reference target');
			}
			return target;
		};
		class WeakRef {
			/** @type {unknown} */
			#target;
			/** @param {unknown} target */
			constructor(target) {
				this.#target = collectable(target);
			}
			deref() {
				return this.#target;
			}
		}
		class FinalizationRegistry {
			#tokens = new Set();
			/** @param {unknown} cleanup */
			constructor(cleanup) {
				if (typeof cleanup !== 'function') {
					throw new TypeError('FinalizationRegistry: cleanup must be callable');
				}
			}
			/**
			 * @param {unknown} target
			 * @param {unknown} held
			 * @param {unknown} [token]
			 */
			register(target, held, token) {
				if (collectable(target) === held) {
					throw new TypeError('FinalizationRegistry: target and holdings must not be the same');
				}
				if (token !== undefined) this.#tokens.add(token);
			}
			/** @param {unknown} token */
			unregister(token) {
				return this.#tokens.delete(token);
			}
		}
		for (const Inert of [WeakRef, FinalizationRegistry]) {
			Object.defineProperty(Inert.prototype, Symbol.toStringTag, {
				value: Inert.name,
				configurable: true
			});
			replace_member(global, Inert.name, () => Inert);
		}
	}

	// the console's timers print nothing, so stderr is the same each run
	for (const name of ['time', 'timeLog', 'timeEnd']) {
		replace_member(
			global.console,
			name,
			() => /** @type {Record<string, () => void>} */ ({ [name]() {} })[name]
		);
	}

	// no timers at all: late work runs as promise jobs
	delete global.setTimeout;
	delete global.setInterval;
	delete global.clearTimeout;
	delete global.clearInterval;

	// no blobs: a `File` holds the clock and a blob part a random id, each
	// in state its class keeps, and the object URLs exist only for blobs
	delete global.Blob;
	delete global.File;
	delete url_ns.URL.createObjectURL;
	delete url_ns.URL.revokeObjectURL;
};
