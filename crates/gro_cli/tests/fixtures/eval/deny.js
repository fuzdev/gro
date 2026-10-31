// every operation `determinism: deny` refuses, each called, then every way
// back to its original tried: the result per entry is the refusal's message
// and the ways that reached a working original
const refused = (call) => {
	try {
		call();
		return 'ran';
	} catch (error) {
		return error instanceof Error ? error.message : String(error);
	}
};

// whether `call` now runs without throwing: a working original reached
const runs = (call) => {
	try {
		call();
		return true;
	} catch {
		return false;
	}
};

const Performance = Object.getPrototypeOf(performance);
const descriptor = (owner, name) => Object.getOwnPropertyDescriptor(owner, name);

// [entry, the refused call, the ways back to the original, each a call that
// runs only if it reached one]
const entries = [
	[
		'Math.random',
		() => Math.random(),
		{
			descriptor: () => descriptor(Math, 'random').value(),
			reflect: () => Reflect.apply(Reflect.get(Math, 'random'), Math, [])
		}
	],
	...['now', 'mark', 'measure', 'toJSON'].map((name) => [
		`performance.${name}`,
		() => performance[name]('a'),
		{
			prototype: () => Performance[name].call(performance, 'a'),
			own: () => {
				if (Object.hasOwn(performance, name)) throw new Error('an own member');
				return performance[name]('a');
			}
		}
	]),
	[
		'performance.timeOrigin',
		() => performance.timeOrigin,
		{ getter: () => descriptor(Performance, 'timeOrigin').get.call(performance) }
	],
	...Object.getOwnPropertyNames(Temporal.Now).map((name) => [
		`Temporal.Now.${name}`,
		() => Temporal.Now[name](),
		{ descriptor: () => descriptor(Temporal.Now, name).value() }
	]),
	[
		'Date()',
		() => Date(),
		{
			constructor: () => new Date(0).constructor(),
			prototype: () => Date.prototype.constructor()
		}
	],
	[
		'new Date()',
		() => new Date(),
		{
			constructor: () => new new Date(0).constructor(),
			prototype: () => new Date.prototype.constructor(),
			reflect: () => Reflect.construct(Date, []),
			subclass: () => new (class extends Date {})(),
			global: () => new (descriptor(globalThis, 'Date').value)()
		}
	],
	[
		'Date.now',
		() => Date.now(),
		{
			constructor: () => new Date(0).constructor.now(),
			descriptor: () => descriptor(Date, 'now').value()
		}
	],
	[
		'Intl.DateTimeFormat format',
		() => new Intl.DateTimeFormat().format(),
		{
			getter: () =>
				descriptor(Intl.DateTimeFormat.prototype, 'format').get.call(new Intl.DateTimeFormat())(),
			// `undefined` formats the present, as no argument does
			undefined: () => new Intl.DateTimeFormat().format(undefined)
		}
	],
	[
		'Intl.DateTimeFormat formatToParts',
		() => new Intl.DateTimeFormat().formatToParts(),
		{
			descriptor: () =>
				descriptor(Intl.DateTimeFormat.prototype, 'formatToParts').value.call(
					new Intl.DateTimeFormat()
				),
			undefined: () => new Intl.DateTimeFormat().formatToParts(undefined)
		}
	],
	[
		'AbortSignal.timeout',
		() => AbortSignal.timeout(1),
		{ descriptor: () => descriptor(AbortSignal, 'timeout').value(1) }
	],
	[
		'Atomics.waitAsync',
		() => Atomics.waitAsync(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1),
		{
			descriptor: () =>
				descriptor(Atomics, 'waitAsync').value(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1),
			// a timeout that coerces to a finite number is refused too
			coerced: () =>
				Atomics.waitAsync(new Int32Array(new SharedArrayBuffer(4)), 0, 0, { valueOf: () => 1 })
		}
	]
];

const result = {};
for (const [entry, call, ways] of entries) {
	result[entry] = {
		refused: refused(call),
		recovered: Object.keys(ways).filter((way) => runs(ways[way]))
	};
}

// deleting a replacement leaves nothing, never the original
const deleted = [
	[Math, 'random'],
	[Performance, 'now'],
	[Temporal.Now, 'instant'],
	[Date, 'now']
];
result.deleted = deleted
	.filter(([owner, name]) => {
		delete owner[name];
		return typeof owner[name] === 'function' && runs(() => owner[name]());
	})
	.map(([, name]) => name);

// absent, with every name a user could reach them by: `Blob` and `File`
// keep the clock and randomness in their instances' own state, and the object
// URLs exist only for blobs
result.absent = [
	typeof Blob,
	typeof File,
	typeof URL.createObjectURL,
	typeof URL.revokeObjectURL,
	'Blob' in globalThis,
	'File' in globalThis
];

// what's kept: dates from values, a format given a date, a wait that can't
// wait, and the attributes each replaced member had
const ia = new Int32Array(new SharedArrayBuffer(4));
result.kept = {
	date: new Date(0).toISOString(),
	utc: Date.UTC(2026, 0, 1),
	parse: Date.parse('2026-01-01T00:00:00Z'),
	format: new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).format(0),
	instanceof: new Date(0) instanceof Date,
	date_name: `${Date.name}/${Date.length}`,
	wait_async_now: Atomics.waitAsync(ia, 0, 1, 0).value,
	wait_async_zero: Atomics.waitAsync(ia, 0, 0, 0).value,
	attributes: [
		descriptor(globalThis, 'Date'),
		descriptor(Date.prototype, 'constructor'),
		descriptor(Intl.DateTimeFormat.prototype, 'format')
	].map((d) => `${d.writable}/${d.enumerable}/${d.configurable}`)
};

// the garbage collector, unobservable: a `WeakRef` that always derefs, and
// a registry whose callback never runs; and the console's timers, silent
const target = {};
const registry = new FinalizationRegistry(() => {
	throw new Error('a finalizer ran');
});
registry.register({}, 'held', target);
console.time('label');
console.timeLog('label');
console.timeEnd('label');
result.inert = {
	deref: new WeakRef(target).deref() === target,
	weak_ref_tag: Object.prototype.toString.call(new WeakRef(target)),
	registry_tag: Object.prototype.toString.call(registry),
	unregister: registry.unregister(target),
	weak_ref_refuses: refused(() => new WeakRef(1)),
	console_time: `${console.time.name}/${console.timeLog.name}/${console.timeEnd.name}`
};

export default result;
