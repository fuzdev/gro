// The first import of every builtin, started in each frame from near the stack's limit up, all in
// one turn. `import()` of a builtin no module has imported yet runs the builtin's backing script
// on the caller's stack, so with too little stack left the module is built from the event loop
// instead, and the import settles then; nearest the limit the import's promise rejects with the
// engine's own `RangeError`, when `deno_core` can't make its handler on the import's promise. For
// every builtin (`modules.js`, which the test writes from the builtin table), this counts the
// imports that settled after one started above them did, which were built from the event loop,
// counts the engine's refusals, keeps any other failure as it reads, and prints the exports of
// the module and whether every import, and one made later, is that same module.

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
 * before, and records the order they settle in.
 * @param {string} specifier
 * @returns {{settled: Array<Promise<{namespace: any} | {error: unknown}>>, order: Array<number>}}
 * how each import settles, the deepest first, and each one's place in that list as it settles
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
	/** @type {Array<number>} */
	const order = [];
	// handled before the run's next turn, which would report a rejection still unhandled
	const settled = started.map((promise, index) =>
		promise.then(
			(namespace) => {
				order.push(index);
				return { namespace };
			},
			(error) => {
				order.push(index);
				return { error };
			}
		)
	);
	return { settled, order };
};

/**
 * How many of the imports settled after one started above them, as `order` lists them: each one
 * built from the event loop.
 * @param {Array<number>} order
 * @returns {number}
 */
const count_late = (order) => {
	let late = 0;
	let highest = -1;
	for (const index of order) {
		if (index < highest) late++;
		else highest = index;
	}
	return late;
};

/**
 * Sweeps every builtin's first import up from the stack's limit and prints the rows.
 */
export const sweep = async () => {
	const rows = [];
	for (const specifier of SPECIFIERS) {
		let engine = 0;
		/** @type {Array<string>} */
		const other = [];
		let whole;
		let same = true;
		const { settled, order } = import_up_from_the_limit(specifier);
		for (const each of settled) {
			const result = await each;
			if ('namespace' in result) {
				if (whole === undefined) whole = result.namespace;
				else same &&= result.namespace === whole;
				continue;
			}
			const { error } = result;
			const message = error instanceof Error ? error.message : '';
			if (error instanceof RangeError && message === 'Maximum call stack size exceeded') {
				engine++;
				continue;
			}
			const read = String(error instanceof Error ? `${error.name}: ${message}` : error);
			if (!other.includes(read)) other.push(read);
		}
		const again = await import(specifier);
		rows.push({
			specifier,
			late: count_late(order),
			engine,
			other,
			exports: whole === undefined ? null : Object.keys(whole).sort(),
			same: same && again === whole
		});
	}
	console.log(JSON.stringify(rows));
};
