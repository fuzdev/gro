// The first import of every builtin, started in each frame from near the stack's limit up, until
// one succeeds. `import()` of a builtin no module has imported yet runs the builtin's
// implementation on the caller's stack, so the loader refuses one with less stack left than that
// needs, before anything of it runs; nearest the limit the import's promise rejects with the
// engine's own `RangeError` instead, when `deno_core` can't make its handler on the promise of
// an import the loader refused. For every builtin (`modules.js`, which the test writes from the
// builtin table), this counts the imports refused either way, keeps any other failure as it
// reads, and prints the exports of the import that succeeded and whether every later import is
// that same module.

import { SPECIFIERS } from './modules.js';

/**
 * How many frames above the deepest one the stack allows start no import: one there could
 * overflow in the call that keeps its promise, leaving its rejection unhandled.
 */
const SKIPPED = 100;

/** How many imports are started, a frame apart, above the skipped frames. */
const HIGHEST = 4000;

/**
 * Starts `import(specifier)` in each of the [`HIGHEST`] frames above the [`SKIPPED`] nearest the
 * deepest one the stack allows, the deepest first, so each has a frame's stack more than the one
 * before.
 * @param {string} specifier
 * @returns {Array<Promise<{namespace: any} | {error: unknown}>>} how each import settles, the
 * deepest first
 */
const import_up_from_the_limit = (specifier) => {
	/** @type {Array<Promise<any>>} */
	const started = [];
	let from_the_limit = 0;
	const dive = () => {
		try {
			dive();
		} catch (error) {
			if (from_the_limit++ >= SKIPPED && started.length < HIGHEST) {
				started.push(import(specifier));
			}
			throw error;
		}
	};
	try {
		dive();
	} catch {
		// the overflow that found the limit
	}
	// handled before the run's next turn, which would report a rejection still unhandled
	return started.map((promise) =>
		promise.then(
			(namespace) => ({ namespace }),
			(error) => ({ error })
		)
	);
};

/**
 * Which refusal `error` is: `loader`, the loader's of an `import()` of `specifier` short of
 * stack, a `TypeError` with the code of every `import()` that fails to resolve or load; `engine`, the engine's
 * `RangeError`; or anything else, as it reads.
 * @param {unknown} error
 * @param {string} specifier
 * @returns {string}
 */
const to_refusal = (error, specifier) => {
	const message = error instanceof Error ? error.message : '';
	if (error instanceof RangeError && message === 'Maximum call stack size exceeded') {
		return 'engine';
	}
	if (
		error instanceof TypeError &&
		/** @type {{code?: unknown}} */ (error).code === 'ERR_MODULE_NOT_FOUND' &&
		message.startsWith(`cannot import '${specifier}' from `) &&
		message.includes(': Maximum call stack size exceeded: an import() of a builtin module needs ')
	) {
		return 'loader';
	}
	return String(error instanceof Error ? `${error.name}: ${message}` : error);
};

/**
 * Sweeps every builtin's first import up from the stack's limit and prints the rows.
 */
export const sweep = async () => {
	const rows = [];
	for (const specifier of SPECIFIERS) {
		/** @type {Record<string, number>} */
		const refused = { loader: 0, engine: 0 };
		/** @type {Array<string>} */
		const other = [];
		let whole;
		let same = true;
		for (const started of import_up_from_the_limit(specifier)) {
			const settled = await started;
			if ('namespace' in settled) {
				if (whole === undefined) whole = settled.namespace;
				else same &&= settled.namespace === whole;
				continue;
			}
			const refusal = to_refusal(settled.error, specifier);
			// every import after the first that succeeded finds the module made
			if (whole === undefined && refusal in refused) refused[refusal]++;
			else if (!other.includes(refusal)) other.push(refusal);
		}
		const again = await import(specifier);
		rows.push({
			specifier,
			refused,
			other,
			exports: whole === undefined ? null : Object.keys(whole).sort(),
			same: same && again === whole
		});
	}
	console.log(JSON.stringify(rows));
};
