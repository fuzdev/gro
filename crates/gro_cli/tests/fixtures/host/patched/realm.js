// The realm, patched: every intrinsic a program can reach replaced by a trap that records its
// use while armed, so a run can show that a builtin module's implementation, which the runtime
// compiles and runs at the builtin's first import, uses none of them, at its top level or in
// what it exports. A trap is transparent: it behaves as what it replaced, armed or not, so the
// program around it, and the runtime beneath it, keep working, and only the record tells.
//
// A trap's own body calls only functions taken before the realm was patched.

const { apply, defineProperty, deleteProperty, getOwnPropertyDescriptor, getPrototypeOf, ownKeys } =
	Reflect;
const { setPrototypeOf } = Reflect;
const { hasOwn, isFrozen } = Object;
const to_string = String;
const stringify = JSON.stringify;
const print = console.log;

/** The prototypes a host module's values have, by name. */
const PROTOTYPES = [
	['Array.prototype', Array.prototype],
	['Error.prototype', Error.prototype],
	['Function.prototype', Function.prototype],
	['Object.prototype', Object.prototype]
];

/** The well-known symbols, the protocols a value of any type can be made to answer. */
const PROTOCOLS = ownKeys(Symbol)
	.map((key) => /** @type {any} */ (Symbol)[key])
	.filter((value) => typeof value === 'symbol');

/** The prototypes every object, function, and array reads a property it lacks from. */
const SHARED_PROTOTYPES = [
	['Object.prototype', Object.prototype],
	['Function.prototype', Function.prototype],
	['Array.prototype', Array.prototype]
];

/**
 * Each replaced property, to put back: its owner, its key, and what it was, if anything.
 * @type {Array<[object, string | symbol, PropertyDescriptor | undefined]>}
 */
const replaced = [];

let armed = false;
let recorded = '';

/** Starts recording every use of a trap. */
export const arm = () => {
	armed = true;
};

/** Stops recording. */
export const disarm = () => {
	armed = false;
};

/** The traps used while armed, each as `get` or `set` and the path of its property. */
export const read_recorded = () => recorded;

/** How many properties are replaced. */
export const count_replaced = () => replaced.length;

/**
 * Calls `fn` with `receiver` and no arguments, by a function taken before the realm was
 * patched, so the call itself uses no trap.
 * @param {Function} fn
 * @param {unknown} receiver
 * @returns {unknown}
 */
export const call = (fn, receiver) => apply(fn, receiver, []);

/**
 * A descriptor with no prototype, so a property definition reads none of its fields through
 * `Object.prototype`, where the traps are.
 * @param {PropertyDescriptor} descriptor
 * @returns {PropertyDescriptor}
 */
const bare = (descriptor) => {
	setPrototypeOf(descriptor, null);
	return descriptor;
};

/**
 * Replaces the property `key` of `owner`, which `descriptor` describes when it has one, with
 * an accessor that records its use as `shown` while armed and behaves as the property did:
 * a read gives what it gave, and a write does what it did.
 * @param {object} owner
 * @param {string | symbol} key
 * @param {PropertyDescriptor | undefined} descriptor
 * @param {string} shown
 */
const trap = (owner, key, descriptor, shown) => {
	if (descriptor !== undefined) bare(descriptor);
	replaced[replaced.length] = [owner, key, descriptor];
	/** @param {any} receiver @param {unknown} value */
	const assign = (receiver, value) => {
		if (receiver === owner && descriptor !== undefined) {
			descriptor.value = value;
		} else if (
			(typeof receiver === 'object' && receiver !== null) ||
			typeof receiver === 'function'
		) {
			defineProperty(
				receiver,
				key,
				bare({ value, writable: true, enumerable: true, configurable: true })
			);
		}
	};
	defineProperty(
		owner,
		key,
		bare({
			get() {
				if (armed) recorded += `get ${shown};`;
				if (descriptor === undefined) return undefined;
				return descriptor.get === undefined ? descriptor.value : apply(descriptor.get, this, []);
			},
			set(value) {
				if (armed) recorded += `set ${shown};`;
				if (descriptor === undefined || descriptor.writable === true) {
					assign(this, value);
				} else if (descriptor.set !== undefined) {
					apply(descriptor.set, this, [value]);
				}
			},
			enumerable: descriptor?.enumerable ?? false,
			configurable: true
		})
	);
};

/**
 * Patches the realm: every configurable property of the global object and of every object
 * reachable from it, the intrinsics' constructors and prototypes among them, becomes a trap;
 * so do `Symbol.hasInstance` and `Symbol.species` on every function reached, which
 * `instanceof` and a species lookup read; and so does each of `names` and each well-known
 * symbol on the prototypes every value shares, where a read or a write of a property an
 * object lacks lands.
 * @param {Array<string>} names - the property names to trap on the shared prototypes
 */
export const patch_realm = (names) => {
	const seen = new Set();
	/** @type {Array<[object, string]>} */
	const reached = [[globalThis, 'globalThis']];
	for (let i = 0; i < reached.length; i++) {
		const [owner, path] = /** @type {[object, string]} */ (reached[i]);
		if (seen.has(owner)) continue;
		seen.add(owner);
		const prototype = getPrototypeOf(owner);
		if (prototype !== null) reached.push([prototype, `${path}.__proto__`]);
		for (const key of ownKeys(owner)) {
			const descriptor = /** @type {PropertyDescriptor} */ (getOwnPropertyDescriptor(owner, key));
			const shown = `${path}.${to_string(key)}`;
			for (const value of [descriptor.value, descriptor.get, descriptor.set]) {
				if ((typeof value === 'object' && value !== null) || typeof value === 'function') {
					reached.push([value, shown]);
				}
			}
			if (descriptor.configurable) trap(owner, key, descriptor, shown);
		}
	}
	for (const owner of seen) {
		if (typeof owner !== 'function') continue;
		for (const symbol of [Symbol.hasInstance, Symbol.species]) {
			if (!hasOwn(owner, symbol))
				trap(owner, symbol, undefined, `a function's ${to_string(symbol)}`);
		}
	}
	for (const [path, prototype] of SHARED_PROTOTYPES) {
		for (const key of [...names, ...PROTOCOLS]) {
			if (!hasOwn(prototype, key)) trap(prototype, key, undefined, `${path}.${to_string(key)}`);
		}
	}
};

/** Puts back every property `patch_realm` replaced, the latest first. */
export const restore_realm = () => {
	for (let i = replaced.length - 1; i >= 0; i--) {
		const [owner, key, descriptor] = /** @type {(typeof replaced)[number]} */ (replaced[i]);
		if (descriptor === undefined) {
			deleteProperty(owner, key);
		} else {
			defineProperty(owner, key, descriptor);
		}
	}
	replaced.length = 0;
};

/**
 * What `value` is, as text: a primitive by its type and value, an object or function by its
 * prototype, whether it's frozen, and each own property, to `depth`, past which a value is
 * shown by its type alone.
 * @param {unknown} value
 * @param {number} [depth]
 * @returns {string}
 */
export const show = (value, depth = 3) => {
	if (value === null) return 'null';
	if (depth === 0) return typeof value;
	if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
		return `${typeof value} ${stringify(value)}`;
	}
	if (typeof value !== 'object' && typeof value !== 'function') return typeof value;
	const prototype = getPrototypeOf(value);
	const name =
		prototype === null
			? 'null'
			: (PROTOTYPES.find(([, intrinsic]) => intrinsic === prototype)?.[0] ?? 'other');
	const properties = ownKeys(value).map((key) => {
		const descriptor = /** @type {PropertyDescriptor} */ (getOwnPropertyDescriptor(value, key));
		const data = hasOwn(descriptor, 'value');
		const flags =
			(descriptor.enumerable ? 'e' : '-') +
			(descriptor.configurable ? 'c' : '-') +
			(data ? (descriptor.writable ? 'w' : '-') : 'a');
		return ` ${to_string(key)} [${flags}]: ${data ? show(descriptor.value, depth - 1) : 'accessor'};`;
	});
	return `${typeof value} of ${name}${isFrozen(value) ? ', frozen' : ''} {${properties.join('')} }`;
};

/**
 * Prints `line` with the console taken before the realm was patched.
 * @param {string} line
 */
export const print_line = (line) => print(line);
