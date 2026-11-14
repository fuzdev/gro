// Every builtin's first import started short of stack (`sweep.js`), in a realm patched against
// the host's report of a failure: every error class's prototype has a `cause` getter returning
// an aggregate of a long chain of causes, and `Error.prepareStackTrace` formats each call site.
// A first import cut short would be reported by following that chain on the importer's stack.

import { sweep } from './sweep.js';

/** @type {unknown} */
let cause;
for (let i = 0; i < 40; i++) cause = new Error(`cause ${i}`, { cause });
const aggregate = new AggregateError([cause, cause], 'aggregate', { cause });
for (const ErrorClass of [Error, RangeError, TypeError, ReferenceError, SyntaxError, EvalError]) {
	Object.defineProperty(ErrorClass.prototype, 'cause', {
		get: () => aggregate,
		configurable: true
	});
}
/** @type {any} */ (Error).prepareStackTrace = (
	/** @type {unknown} */ _error,
	/** @type {Array<any>} */ sites
) => sites.map((site) => String(site.getFunctionName())).join('\n');

await sweep();
