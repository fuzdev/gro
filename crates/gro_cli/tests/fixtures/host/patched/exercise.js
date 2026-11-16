// The exerciser: calls everything a value exports or reaches, as a program holding it could,
// and reports each call's outcome. A function is called with no arguments, with each list of
// arguments the caller gives for it, with each argument of such a list in turn replaced by a
// hostile one, with hostile arguments of every kind, and with a hostile receiver; a class is
// constructed the same ways; then the functions its results, what it threw, and its own
// properties hold are exercised in turn, and those of each prototype a builtin made, with the
// value as their receiver, and each getter is read and each setter given a value. The traps
// are armed for the length of a call and no longer, so what a call leaves to run later, a
// promise's reaction, runs unwatched. A call that returns a promise has it handled at once, so
// a rejection is never the run's unhandled one, and reports how it settled once every call is
// made (`settle`).
//
// A call can end the process, as `exit` does. Each call is reported before it's made, so a run
// that ended in one left which, with every trap and touch it used before it did; the test
// runs the entry again with that call among `ended`, the calls to skip, and so on to a run
// that gets to its end.
//
// A hostile argument reports every use made of it during the call it was passed to, a `touch`
// report: a function that reads, coerces, or iterates what it's given runs the giver's code
// from inside the builtin, which is never an accident a test should pass over.
//
// This module is loaded by a run's entry, before the realm is patched: what it takes from the
// realm it takes then, and its hostile values call only those.

import {
	arm,
	call,
	construct_with,
	disarm,
	is_class,
	is_intrinsic,
	name_value,
	report,
	show,
	unwatched
} from './realm.js';

const { apply, defineProperty, getOwnPropertyDescriptor, getPrototypeOf, ownKeys, setPrototypeOf } =
	Reflect;
const { hasOwn } = Object;
const to_string = String;
const ProxyConstructor = Proxy;
const { iterator, toPrimitive } = Symbol;
const array_values = Array.prototype[Symbol.iterator];
const promise_prototype = Promise.prototype;
const promise_then = Promise.prototype.then;
const PromiseConstructor = Promise;
// the engine's own accessor of every error's `stack`, which formats the stack when it's read:
// a use of the realm that is the reader's, not the builtin's that made the error
const error_stack = getOwnPropertyDescriptor(new Error(''), 'stack') ?? {};

/**
 * Each function of `Reflect`, by name: what a proxy's trap of that name does by default.
 * @type {Array<[string, Function]>}
 */
const REFLECT = [];
for (const name of ownKeys(Reflect)) {
	const value = /** @type {any} */ (Reflect)[name];
	if (typeof name === 'string' && typeof value === 'function') REFLECT.push([name, value]);
}

/** The call whose hostile arguments report their uses, if one is under way. */
let watched = '';

/**
 * Reports a use of a hostile argument of the call under way.
 * @param {string} use
 */
const touch = (use) => {
	if (watched !== '') report('touch', `${watched}: ${use}`);
};

class HostileString extends String {
	get cause() {
		touch('string.cause');
		return undefined;
	}
	toString() {
		touch('string.toString');
		return 'hostile';
	}
	valueOf() {
		touch('string.valueOf');
		return 'hostile';
	}
	[toPrimitive]() {
		touch('string[Symbol.toPrimitive]');
		return 'hostile';
	}
}

class HostileArray extends Array {
	get cause() {
		touch('array.cause');
		return undefined;
	}
	[iterator]() {
		touch('array[Symbol.iterator]');
		return apply(array_values, this, []);
	}
	[toPrimitive]() {
		touch('array[Symbol.toPrimitive]');
		return 'hostile';
	}
}

/**
 * A proxy of `target` whose every trap reports, as `kind`, and then does what it would have
 * without the trap.
 * @param {string} kind
 * @param {object} target
 */
const to_reporting_proxy = (kind, target) => {
	/** @type {Record<string, Function>} */
	const handler = { __proto__: null };
	for (const [name, forward] of REFLECT) {
		handler[name] = (/** @type {Array<any>} */ ...args) => {
			// a trap of one argument has no key, and a read past its end would use the realm
			const key = args.length > 1 ? args[1] : undefined;
			const named = typeof key === 'string' || typeof key === 'symbol';
			touch(`${kind}.${name}${named ? ` ${to_string(key)}` : ''}`);
			// what the trap does by default is the caller's code, as the trap is
			return unwatched(() => apply(forward, undefined, args));
		};
	}
	return new ProxyConstructor(target, handler);
};

/** A function whose prototype is `null`, so a read it lacks finds nothing of the realm's. */
const null_prototype_function = () => {
	const fn = function () {};
	setPrototypeOf(fn, null);
	return fn;
};

/**
 * The hostile arguments, by kind, each made new for a call: a proxy, whose every trap
 * reports; a proxy of a function, which a check of its type takes for a callback, and whose
 * call reports; an object whose conversions, `then`, `length`, first index, iterator, and
 * `cause` are getters that report; a subclassed string, whose conversions and `cause` report;
 * a subclassed array, whose iterator, conversion, and `cause` report; and one of those whose
 * first element is a getter that reports, which a read of the array's elements runs. What a
 * proxy's trap does by default runs unwatched, it's the caller's code, and a proxy's target has a `null`
 * prototype, so it finds nothing of the realm's, and every read a builtin makes of one of
 * these is the argument's to answer, never a trap's on the realm's prototypes. Each is named for `show`, which shows it by its name, never walking
 * it, when a builtin hands it back inside what it returns.
 * @type {Array<[string, () => unknown]>}
 */
const HOSTILE = /** @type {Array<[string, () => unknown]>} */ ([
	['proxy', () => to_reporting_proxy('proxy', { __proto__: null })],
	['callable', () => to_reporting_proxy('callable', null_prototype_function())],
	[
		'getters',
		() => {
			const hostile = {};
			for (const key of [
				'toString',
				'valueOf',
				toPrimitive,
				'then',
				'length',
				'0',
				iterator,
				'cause'
			]) {
				defineProperty(hostile, key, {
					__proto__: null,
					get() {
						touch(`getters.${to_string(key)}`);
						return undefined;
					}
				});
			}
			return hostile;
		}
	],
	['string', () => new HostileString('hostile')],
	['array', () => HostileArray.of('hostile')],
	[
		'indexed',
		() => {
			const hostile = HostileArray.of('hostile');
			defineProperty(hostile, '0', {
				__proto__: null,
				get() {
					touch('indexed.0');
					return 'hostile';
				}
			});
			return hostile;
		}
	]
]).map(([kind, make]) => [
	kind,
	() => name_value(/** @type {object} */ (make()), `hostile ${kind}`)
]);

/** How many hostile arguments a hostile call passes at least, and as many as a function declares. */
const HOSTILE_ARITY = 3;

/**
 * Exercises the exports of builtins, reporting as it goes: a `call` before each call, a `line`
 * for its outcome, a `touch` for each use of a hostile argument, and an `unexercised` for each
 * function or class reached that `exercises` has no entry for: how many parameters a function
 * declares says nothing of the arguments it reads (a default, a rest), so whoever adds one
 * says what it's called with, an empty list when it takes nothing.
 * @param {Map<string, Array<Array<unknown>>>} exercises - the lists of arguments to call a
 * function or construct a class with, by its path
 * @param {number} depth - how far into a value's properties and its calls' results the
 * functions are looked for
 * @param {ReadonlyArray<string>} ended - the calls an earlier run of the same entry ended the
 * process in, each by its label, which this run skips
 */
export const create_exerciser = (exercises, depth, ended) => {
	/**
	 * The objects walked, and each function with the receivers it was called with.
	 * @type {Map<object, Set<unknown>>}
	 */
	const seen = new Map();
	/** @type {Set<string>} */
	const used = new Set();
	/**
	 * The promises calls returned, each by its call's label, to report how each settled.
	 * @type {Array<[string, Promise<unknown>]>}
	 */
	const promises = [];

	/**
	 * Runs `use` armed, as the call named `label`, and reports its outcome, or that an earlier
	 * run ended in it.
	 * @param {string} label
	 * @param {() => unknown} use
	 * @returns {[returned: boolean, value: unknown]}
	 */
	const attempt = (label, use) => {
		if (ended.includes(label)) {
			report('line', `${label}: ended the process`);
			return [true, undefined];
		}
		report('call', label);
		/** @type {[boolean, unknown]} */
		let outcome;
		watched = label;
		arm();
		try {
			outcome = [true, use()];
		} catch (error) {
			outcome = [false, error];
		}
		disarm();
		watched = '';
		report('line', `${label}: ${outcome[0] ? 'returned' : 'threw'} ${show(outcome[1])}`);
		const [, value] = outcome;
		if (
			typeof value === 'object' &&
			value !== null &&
			getPrototypeOf(value) === promise_prototype
		) {
			// handled now, by the realm's own `then` taken at load, so its rejection is no
			// unhandled one of the run's
			const ignore = () => {};
			apply(promise_then, value, [ignore, ignore]);
			promises.push([label, /** @type {Promise<unknown>} */ (value)]);
		}
		return outcome;
	};

	/**
	 * Calls the function or constructs the class `fn`, each way, and exercises what the calls
	 * with no hostile argument return.
	 * @param {string} path
	 * @param {Function} fn
	 * @param {unknown} receiver
	 * @param {number} left
	 */
	const exercise_function = (path, fn, receiver, left) => {
		const constructed = is_class(fn);
		const shown = constructed ? `new ${path}` : path;
		/** @type {(args: Array<unknown>) => unknown} */
		const use = constructed
			? (args) => construct_with(fn, args)
			: (args) => call(fn, receiver, args);
		const given = exercises.get(path);
		if (given === undefined) {
			report('unexercised', path);
		} else {
			used.add(path);
		}
		const lists = [[], ...(given ?? [])];
		for (let i = 0; i < lists.length; i++) {
			const args = /** @type {Array<unknown>} */ (lists[i]);
			const label = i === 0 ? `${shown}()` : `${shown}(arguments ${i})`;
			const [returned, value] = attempt(label, () => use(args));
			// what a call threw is the builtin's too, when it made it
			exercise(returned ? label : `${label} thrown`, value, undefined, left - 1);
			// each given argument in turn hostile, the others as given, so a check of one
			// argument doesn't keep the hostile ones from the code past it
			for (let at = 0; at < args.length; at++) {
				for (const [kind, make] of HOSTILE) {
					const mixed = [];
					for (let j = 0; j < args.length; j++) mixed[j] = j === at ? make() : args[j];
					attempt(`${shown}(arguments ${i}, ${kind} at ${at})`, () => use(mixed));
				}
			}
		}
		const arity = fn.length > HOSTILE_ARITY ? fn.length : HOSTILE_ARITY;
		for (const [kind, make] of HOSTILE) {
			const args = [];
			for (let i = 0; i < arity; i++) args[i] = make();
			attempt(`${shown}(${kind})`, () => use(args));
		}
		if (!constructed) {
			const hostile = /** @type {[string, () => unknown]} */ (HOSTILE[0])[1]();
			attempt(`${path}.call(proxy)`, () => call(fn, hostile, []));
		}
	};

	/**
	 * Exercises `value`: calls it when it's a function, then exercises what its own
	 * properties hold and what each prototype a builtin made holds, reading each getter.
	 * @param {string} path - what names `value`, the builtin's specifier and the way to it
	 * @param {any} value
	 * @param {unknown} receiver - the object `value` was reached through, if any
	 * @param {number} left - how much deeper to look
	 */
	const exercise = (path, value, receiver, left) => {
		if ((typeof value !== 'object' && typeof value !== 'function') || value === null) return;
		if (left === 0 || is_intrinsic(value)) return;
		const receivers = seen.get(value);
		// a class has no receiver, so it's constructed where it's first reached
		const constructed = typeof value === 'function' && is_class(value);
		if (receivers !== undefined && (receivers.has(receiver) || constructed)) return;
		if (receivers === undefined) {
			seen.set(value, new Set([receiver]));
		} else {
			receivers.add(receiver);
		}
		// a function is called for each receiver it's reached through
		if (typeof value === 'function') exercise_function(path, value, receiver, left);
		if (receivers !== undefined) return;
		for (let owner = value; owner !== null && !is_intrinsic(owner); owner = getPrototypeOf(owner)) {
			for (const key of ownKeys(owner)) {
				const descriptor = getOwnPropertyDescriptor(owner, key);
				if (descriptor === undefined) continue;
				const member = `${path}.${to_string(key)}`;
				if (hasOwn(descriptor, 'value')) {
					exercise(member, descriptor.value, value, left - 1);
				} else {
					const { get, set } = descriptor;
					if (get !== undefined && get === error_stack.get) continue;
					if (get !== undefined) {
						const [returned, got] = attempt(`get ${member}`, () => call(get, value, []));
						if (returned) exercise(member, got, value, left - 1);
					}
					if (set !== undefined) {
						attempt(`set ${member}`, () => call(set, value, [undefined]));
						for (const [kind, make] of HOSTILE) {
							// made before the traps are armed, since making one uses the realm
							const hostile = make();
							attempt(`set ${member} = ${kind}`, () => call(set, value, [hostile]));
						}
					}
				}
			}
		}
	};

	return {
		exercise,
		/** The paths `exercises` has arguments for that no function or class was reached at. */
		unused: () => Array.from(exercises.keys()).filter((path) => !used.has(path)),
		/**
		 * Reports how each promise a call returned settled, in the order the calls were made,
		 * once the realm is restored: a builtin settles its promises before it returns them.
		 */
		settle: async () => {
			for (const [label, promise] of promises) {
				const [how, value] = await new PromiseConstructor((resolve) =>
					apply(promise_then, promise, [
						(/** @type {unknown} */ value) => resolve(['fulfilled', value]),
						(/** @type {unknown} */ error) => resolve(['rejected', error])
					])
				);
				report('line', `${label} settled: ${how} ${show(value)}`);
			}
		}
	};
};
