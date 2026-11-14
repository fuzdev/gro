// Imports every builtin module (`modules.js`, which the test writes from the builtin table),
// calls every function one exports or reaches (a method of an object it exports, or of what a
// call returns), and prints what each namespace, export, and call's result is, with what the
// realm's traps recorded and the functions it didn't call. Loaded by `import()` from an entry
// that armed the traps first, the builtins' implementations ran armed: this module's imports
// are their first.

import { MODULES } from './modules.js';
import {
	arm,
	call,
	count_replaced,
	disarm,
	print_line,
	read_recorded,
	restore_realm,
	show
} from './realm.js';

disarm();

/** @type {Array<string>} */
const lines = [];

/**
 * The functions reached and not called: one that declares parameters, which a call without
 * arguments wouldn't exercise. The test fails on any, until this module calls it with them.
 * @type {Array<string>}
 */
const uncalled = [];

/** How far into an export's properties and its calls' results the functions are looked for. */
const DEPTH = 3;

const seen = new Set();

/**
 * Calls `value` armed when it's a function, with `holder` as its receiver, and every function
 * its result or, when it's an object, its own properties hold, to `depth`, showing each
 * call's outcome.
 * @param {string} path - what names `value`, the builtin's specifier and the way to it
 * @param {any} value
 * @param {unknown} holder - the object `value` is a property of, if any
 * @param {number} depth
 */
const exercise = (path, value, holder, depth) => {
	if ((typeof value !== 'object' && typeof value !== 'function') || value === null) return;
	if (depth === 0 || seen.has(value)) return;
	seen.add(value);
	if (typeof value === 'function') {
		if (value.length > 0) {
			uncalled.push(path);
			return;
		}
		let outcome;
		arm();
		try {
			outcome = ['returned', call(value, holder)];
		} catch (error) {
			outcome = ['threw', error];
		}
		disarm();
		lines.push(`${path}(): ${outcome[0]} ${show(outcome[1])}`);
		if (outcome[0] === 'returned') exercise(`${path}()`, outcome[1], undefined, depth - 1);
		return;
	}
	for (const key of Reflect.ownKeys(value)) {
		const descriptor = Reflect.getOwnPropertyDescriptor(value, key);
		if (descriptor === undefined || !('value' in descriptor)) continue;
		exercise(`${path}.${String(key)}`, descriptor.value, value, depth - 1);
	}
};

for (const [specifier, exports, namespace] of MODULES) {
	// the namespace's own shape, each export shown below
	lines.push(`${specifier}: ${show(namespace, 1)}`);
	for (const [name, per_process] of exports) {
		const value = namespace[name];
		// a value that is the process's own is shown by its type
		lines.push(`${specifier} ${name}: ${per_process ? typeof value : show(value)}`);
		exercise(`${specifier} ${name}`, value, undefined, DEPTH);
	}
}

const replaced = count_replaced();
const recorded = read_recorded();
restore_realm();
print_line(JSON.stringify({ replaced, recorded, lines, uncalled }));
