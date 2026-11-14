// The exerciser: calls everything a value exports or reaches, as a program holding it could,
// and reports each call's outcome. A function is called with no arguments, with each list of
// arguments the caller gives for it, with hostile arguments of every kind, and with a hostile
// receiver; a class is constructed the same ways; then the functions its results and its own
// properties hold are exercised in turn, and those of each prototype a builtin made, with the
// value as their receiver, and each getter is read. The traps are armed for the length of a
// call and no longer.
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
	report,
	show
} from './realm.js';

const { apply, defineProperty, getOwnPropertyDescriptor, getPrototypeOf, ownKeys } = Reflect;
const { hasOwn } = Object;
const to_string = String;
const ProxyConstructor = Proxy;
const { iterator, toPrimitive } = Symbol;
const array_values = Array.prototype[Symbol.iterator];

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
	[iterator]() {
		touch('array[Symbol.iterator]');
		return apply(array_values, this, []);
	}
}

/**
 * The hostile arguments, by kind, each made new for a call: a proxy, whose every trap
 * reports; an object whose conversions, `then`, `length`, first index, and iterator are
 * getters that report; a subclassed string, whose conversions report; and a subclassed array,
 * whose iterator reports.
 * @type {Array<[string, () => unknown]>}
 */
const HOSTILE = [
	[
		'proxy',
		() => {
			/** @type {Record<string, Function>} */
			const handler = { __proto__: null };
			for (const [name, forward] of REFLECT) {
				handler[name] = (/** @type {Array<any>} */ ...args) => {
					const key = args[1];
					const named = typeof key === 'string' || typeof key === 'symbol';
					touch(`proxy.${name}${named ? ` ${to_string(key)}` : ''}`);
					return apply(forward, undefined, args);
				};
			}
			return new ProxyConstructor({}, handler);
		}
	],
	[
		'getters',
		() => {
			const hostile = {};
			for (const key of ['toString', 'valueOf', toPrimitive, 'then', 'length', '0', iterator]) {
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
	['array', () => HostileArray.of('hostile')]
];

/** How many hostile arguments a hostile call passes, more than a function is likely to take. */
const HOSTILE_ARITY = 3;

/**
 * Exercises the exports of builtins, reporting as it goes: a `line` for each call's outcome,
 * a `touch` for each use of a hostile argument, and an `unexercised` for each function or
 * class that declares parameters and has no arguments in `exercises`, which a call without
 * them wouldn't exercise.
 * @param {Map<string, Array<Array<unknown>>>} exercises - the lists of arguments to call a
 * function or construct a class with, by its path
 * @param {number} depth - how far into a value's properties and its calls' results the
 * functions are looked for
 */
export const create_exerciser = (exercises, depth) => {
	/**
	 * The objects walked, and each function with the receivers it was called with.
	 * @type {Map<object, Set<unknown>>}
	 */
	const seen = new Map();
	/** @type {Set<string>} */
	const used = new Set();

	/**
	 * Runs `use` armed, as the call named `label`, and reports its outcome.
	 * @param {string} label
	 * @param {() => unknown} use
	 * @returns {[returned: boolean, value: unknown]}
	 */
	const attempt = (label, use) => {
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
			if (fn.length > 0) report('unexercised', path);
		} else {
			used.add(path);
		}
		const lists = [[], ...(given ?? [])];
		for (let i = 0; i < lists.length; i++) {
			const args = /** @type {Array<unknown>} */ (lists[i]);
			const label = i === 0 ? `${shown}()` : `${shown}(arguments ${i})`;
			const [returned, value] = attempt(label, () => use(args));
			if (returned) exercise(label, value, undefined, left - 1);
		}
		for (const [kind, make] of HOSTILE) {
			const args = [];
			for (let i = 0; i < HOSTILE_ARITY; i++) args[i] = make();
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
				} else if (descriptor.get !== undefined) {
					const { get } = descriptor;
					const [returned, got] = attempt(`get ${member}`, () => call(get, value, []));
					if (returned) exercise(member, got, value, left - 1);
				}
			}
		}
	};

	return {
		exercise,
		/** The paths `exercises` has arguments for that no function or class was reached at. */
		unused: () => Array.from(exercises.keys()).filter((path) => !used.has(path))
	};
};
