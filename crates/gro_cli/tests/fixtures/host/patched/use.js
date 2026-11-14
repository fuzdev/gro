// Imports every builtin module (`modules.js`, which the test writes from the builtin table),
// exercises every export (`exercise.js`: each function called with no arguments, with the
// arguments `EXERCISES` gives for it, with hostile arguments, and with a hostile receiver, a
// class constructed, a getter read, a setter given a value, and the same for what each
// returns, throws, or holds), and reports what each namespace, export, and call's outcome is,
// as it goes. Loaded by `import()` from an entry that armed the traps first, the builtins'
// implementations ran armed: this module's imports are their first.

import { MODULES } from './modules.js';
import { create_exerciser } from './exercise.js';
import { count_replaced, disarm, report, restore_realm, show } from './realm.js';

disarm();

/**
 * The arguments each function or class a builtin exports or reaches is exercised with, by its
 * path as the reports name it (`<specifier> <export>`, then `.member` or `()` for each step):
 * lists that reach what a call with no argument or with hostile ones doesn't, and none for a
 * function that takes nothing. A function reached that has no entry here fails the test,
 * whatever it declares, and so does an entry nothing was reached at.
 * @type {Map<string, Array<Array<unknown>>>}
 */
const EXERCISES = new Map([
	// takes no argument and no receiver
	['@fuzdev/gro/cwd.ts cwd', []]
]);

/** How far into an export's properties and its calls' results the functions are looked for. */
const DEPTH = 4;

const exerciser = create_exerciser(EXERCISES, DEPTH);

for (const [specifier, exports, namespace] of MODULES) {
	// the namespace's own shape, each export shown below
	report('line', `${specifier}: ${show(namespace, 1)}`);
	for (const [name, per_process] of exports) {
		const value = namespace[name];
		// a value that is the process's own is shown by its type
		report('line', `${specifier} ${name}: ${per_process ? typeof value : show(value)}`);
		exerciser.exercise(`${specifier} ${name}`, value, undefined, DEPTH);
	}
}

const replaced = count_replaced();
restore_realm();
report('done', { replaced, unused: exerciser.unused() });
