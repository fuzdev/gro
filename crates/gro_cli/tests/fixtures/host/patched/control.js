// The entry of the control run: in a patched realm it does what a builtin's implementation
// must not, once each and armed, then exercises a namespace of its own whose exports do the
// same from every place the exerciser must reach, and reports as it goes. So a patched run
// that reports no trap, no touch, and no unexercised function was watched.

import { NAMES } from './names.js';
import { create_exerciser } from './exercise.js';
import { arm, disarm, patch_realm, report, restore_realm } from './realm.js';

await null;

class Sub extends Array {}

// made before the realm is patched, as an error a builtin's function returns is made by the
// implementation's own `Error`, with no use of the realm
const made_error = new Error('made');

/**
 * What a builtin might export, each doing from its own place what an implementation must not:
 * `Object`, read as a global, stands for any use of the realm.
 */
class Handle {
	#brand = 1;
	constructor() {
		Object;
	}
	read() {
		return this.#brand;
	}
	get size() {
		Object;
		return 1;
	}
	static open() {
		Object;
	}
}
const probe = {
	// a function that declares a parameter, and uses the realm only when it's given one
	with_parameter: (/** @type {unknown} */ given) => (given === undefined ? 1 : Object),
	// one that makes a string of its argument, and one that iterates it
	coerces: (/** @type {unknown} */ given) => `${given}`,
	iterates: (/** @type {Iterable<unknown>} */ given) => [...given],
	// one that reads an option of its argument
	reads_option: (/** @type {{ recursive?: boolean }} */ options) => options.recursive === true,
	// one that reads an option only past a check of its first argument, one that calls its
	// argument when it's a function, and one that makes a string of its fourth
	guarded: (/** @type {unknown} */ path, /** @type {any} */ options) =>
		typeof path === 'string' && options !== undefined ? options.recursive === true : false,
	calls: (/** @type {unknown} */ given) => (typeof given === 'function' ? given() : undefined),
	fourth: (
		/** @type {unknown} */ _a,
		/** @type {unknown} */ _b,
		/** @type {unknown} */ _c,
		/** @type {unknown} */ d
	) => (d === undefined ? '' : `${d}`),
	// one that asks its argument for its keys, a trap of a proxy that is given no key
	enumerates: (/** @type {object} */ given) => {
		for (const key in given) return key;
		return undefined;
	},
	// one that returns an error, whose stack the exerciser must not read for it
	returns_error: () => made_error,
	// one that throws an object of its own, whose method uses the realm
	throws: () => {
		throw { method: () => Object };
	},
	// an object whose method and getter use the realm, and a function whose result's method does
	holder: {
		method() {
			Object;
		},
		get getter() {
			Object;
			return 1;
		},
		set written(/** @type {unknown} */ value) {
			Object;
		}
	},
	returns_holder: () => ({ method: () => Object }),
	// a class: its constructor, a static method, and a getter use the realm, and a method reads
	// a private field, so only an instance is its receiver
	Handle
};

patch_realm(NAMES);
const record = { args: 1 };
const list = [1, 2];
const sub = new Sub();
/** @type {Array<() => unknown>} */
const uses = [
	// a global read
	() => Object,
	// a method of an intrinsic's constructor and of its prototype
	() => Reflect.ownKeys(record),
	() => list.map((n) => n),
	() => (() => 1).call(undefined),
	// iteration
	() => [...list],
	// a string made of an object
	() => `${record}`,
	// `instanceof`, and a species lookup
	() => record instanceof Error,
	() => sub.slice(),
	// a property an object lacks, read and written through `Object.prototype`
	() => /** @type {any} */ (record).default,
	() => {
		/** @type {any} */ (record).cwd = 1;
	},
	// a thenable lookup, and a descriptor's fields
	() => Promise.resolve(record),
	() => Object.defineProperty(record, 'x', { value: 1 }),
	// an index past an array's end
	() => list[5],
	// the options a web class reads from an object given to it
	() => new TextDecoder('utf-8', {}),
	// the formatter of an error's stack
	() => new Error('made').stack
];
for (let i = 0; i < uses.length; i++) {
	const use = /** @type {() => unknown} */ (uses[i]);
	report('use', i);
	arm();
	try {
		use();
	} catch {
		// a patched realm can fail what it traps, which is reported all the same
	}
	disarm();
}

report('use', 'probe');
const exerciser = create_exerciser(
	new Map([
		['probe.coerces', [['given']]],
		['probe.iterates', [[['given']]]],
		['probe.guarded', [['given', undefined]]],
		['probe.calls', [[undefined]]],
		['probe.enumerates', [[{}]]],
		['probe.fourth', [['a', 'b', 'c', undefined]]],
		// the ones that take nothing: an entry all the same, but for `probe.returns_holder`
		['probe.returns_error', []],
		['probe.throws', []],
		['probe.throws() thrown.method', []],
		['probe.holder.method', []],
		['probe.returns_holder().method', []],
		['probe.Handle', []],
		['new probe.Handle().read', []],
		['probe.Handle.prototype.read', []],
		['probe.Handle.open', []],
		['probe.never_reached', [[]]]
	]),
	4
);
exerciser.exercise('probe', probe, undefined, 4);
restore_realm();
report('done', { unused: exerciser.unused() });
