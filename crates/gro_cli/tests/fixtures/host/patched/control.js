// The entry of the control run: in a patched realm, armed, it does what a builtin's
// implementation must not, once each, and prints what the traps recorded, so a patched run
// that records nothing is known to have been watched.

import { NAMES } from './names.js';
import { arm, disarm, patch_realm, print_line, read_recorded, restore_realm } from './realm.js';

await null;

patch_realm(NAMES);
const record = { args: 1 };
const list = [1, 2];
class Sub extends Array {}
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
	() => Object.defineProperty(record, 'x', { value: 1 })
];
/** @type {Array<string>} */
const seen = [];
for (let i = 0; i < uses.length; i++) {
	const before = read_recorded().length;
	const use = /** @type {() => unknown} */ (uses[i]);
	arm();
	try {
		use();
	} catch {
		// a patched realm can fail what it traps, which is recorded all the same
	}
	disarm();
	seen[i] = read_recorded().slice(before);
}
restore_realm();
print_line(JSON.stringify(seen));
