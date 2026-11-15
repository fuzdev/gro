// Imports every builtin module (`modules.js`, which the test writes from the builtin table),
// exercises every export (`exercise.js`: each function called with no arguments, with the
// arguments `EXERCISES` gives for it, with hostile arguments, and with a hostile receiver, a
// class constructed, a getter read, a setter given a value, and the same for what each
// returns, throws, or holds), and reports what each namespace, export, and call's outcome is,
// as it goes. Loaded by `import()` from an entry that armed the traps first, the builtins'
// implementations ran armed: this module's imports are their first.
//
// A call that ends the process, as `exit`'s do, ends the run there, short of its end: the
// test runs the entry again with that call in `skips.js`, which it writes, until a run gets
// to its end, and holds each call that ended one to the code it exited with.

import { MODULES } from './modules.js';
import { ENDED } from './skips.js';
import { create_exerciser } from './exercise.js';
import { count_replaced, disarm, report, restore_realm, show } from './realm.js';

disarm();

/** The env error row's namespace, whose class makes the errors its guard is given. */
const env_error = /** @type {any} */ (
	MODULES.find(([specifier]) => specifier === '@fuzdev/gro/env_error.ts')
)[2];

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
	['@fuzdev/gro/cwd.ts cwd', []],
	// a kind and a target, and with them a message and the options of its cause
	[
		'@fuzdev/gro/env_error.ts EnvError',
		[
			['not_granted', 'HOME'],
			['not_granted', 'HOME', 'custom', { __proto__: null, cause: 1 }]
		]
	],
	// an error of the class, of its kind and of another, and one that isn't
	[
		'@fuzdev/gro/env_error.ts is_env_error',
		[
			[new env_error.EnvError('not_granted', 'HOME'), 'not_granted'],
			[new env_error.EnvError('not_granted', 'HOME'), 'other'],
			[{ __proto__: null }]
		]
	],
	// a name the run grants that's set, and one that's unset; with a hostile array in its place,
	// `narrow` is given the one name `hostile`, which the run grants too, as each handle here has
	['@fuzdev/gro/env.ts env.get', [['GRO_PATCHED_SET'], ['GRO_PATCHED_UNSET']]],
	['@fuzdev/gro/env.ts env.get_all', []],
	['@fuzdev/gro/env.ts env.narrow', [[['GRO_PATCHED_SET', 'GRO_PATCHED_UNSET', 'hostile']]]],
	// the same of the handle that call narrowed to
	['@fuzdev/gro/env.ts env.narrow(arguments 1).get', [['GRO_PATCHED_SET'], ['hostile']]],
	['@fuzdev/gro/env.ts env.narrow(arguments 1).get_all', []],
	['@fuzdev/gro/env.ts env.narrow(arguments 1).narrow', [[['hostile', 'GRO_PATCHED_SET']]]],
	// a code, which ends the process as a call with none does
	['@fuzdev/gro/exit.ts exit', [[7]]],
	// a code, the one a run that ends on its own exits with
	['@fuzdev/gro/exit.ts set_exit_code', [[0]]]
]);

/** How far into an export's properties and its calls' results the functions are looked for. */
const DEPTH = 4;

const exerciser = create_exerciser(EXERCISES, DEPTH, ENDED);

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
