// A builtin's first imports in one turn, some started deep enough in the stack that its module is
// built from the event loop and others near the top, where the first builds it on the spot:
// however the imports of a builtin interleave, every one gets the one module. Each case takes a
// builtin of its own, so its imports are its builtin's first, and prints whether every import got
// one module and how many of the deep ones settled after a shallow one started after them did,
// which a deep import built from the event loop does. Nearest the stack's limit an import can
// reject with the engine's own `RangeError`, when `deno_core` can't make its handler on its
// promise; those are counted, and any other failure kept as it reads.

/**
 * How many frames above the deepest one the stack allows start no import: one there could
 * overflow in the call that keeps its promise, leaving its rejection unhandled.
 */
const SKIPPED = 100;

/** How many deep imports a case starts, a frame apart, above the skipped frames. */
const SPAN = 200;

/**
 * Starts an import of `specifier` in each of the [`SPAN`] frames above the [`SKIPPED`] nearest
 * the deepest one the stack allows, the deepest first.
 * @param {string} specifier
 * @param {'evaluation' | 'source'} [phase]
 * @returns {Array<Promise<unknown>>}
 */
const deep = (specifier, phase = 'evaluation') => {
	/** @type {Array<Promise<unknown>>} */
	const started = [];
	let from_the_limit = 0;
	const dive = () => {
		try {
			dive();
		} catch (error) {
			if (from_the_limit++ >= SKIPPED && started.length < SPAN) {
				started.push(phase === 'source' ? import.source(specifier) : import(specifier));
			}
			throw error;
		}
	};
	try {
		dive();
	} catch {
		// the overflow that found the limit
	}
	return started;
};

/**
 * Settles the imports of `groups`, each started in the order given, deep or shallow as its name
 * says: what each got, and how many deep imports settled after a shallow one started after them.
 * @param {Array<[string, Array<Promise<unknown>>]>} groups
 * @returns {Promise<{values: Array<unknown>, errors: Array<string>, engine: number, late: number}>}
 */
const settle = async (groups) => {
	/** @type {Array<string>} */
	const order = [];
	const tagged = groups.flatMap(([name, promises]) =>
		promises.map((promise) =>
			promise.then(
				(value) => {
					order.push(name);
					return { value };
				},
				(error) => {
					order.push(name);
					return { error };
				}
			)
		)
	);
	const results = await Promise.all(tagged);
	/** @type {Array<unknown>} */
	const values = [];
	/** @type {Array<string>} */
	const errors = [];
	let engine = 0;
	for (const result of results) {
		if ('value' in result) {
			values.push(result.value);
			continue;
		}
		const { error } = result;
		if (error instanceof RangeError && error.message === 'Maximum call stack size exceeded') {
			engine++;
		} else {
			const read = error instanceof Error ? error.name : String(error);
			if (!errors.includes(read)) errors.push(read);
		}
	}
	const names = groups.map(([name]) => name);
	let late = 0;
	for (let i = 0; i < order.length; i++) {
		const name = /** @type {string} */ (order[i]);
		if (!name.startsWith('deep')) continue;
		const at = names.indexOf(name);
		const overtaken = order
			.slice(0, i)
			.some((before) => before.startsWith('shallow') && names.indexOf(before) > at);
		if (overtaken) late++;
	}
	return { values, errors, engine, late };
};

/**
 * Whether there is a value in `values` and every one is the first.
 * @param {Array<unknown>} values
 * @returns {boolean}
 */
const all_same = (values) => values.length > 0 && values.every((value) => value === values[0]);

const PID = '@fuzdev/gro/pid.ts';
const CWD = '@fuzdev/gro/cwd.ts';
const VERSION = '@fuzdev/gro/runtime_version.ts';
const ENV_ERROR = '@fuzdev/gro/env_error.ts';
const ENV = '@fuzdev/gro/env.ts';
const ARGS = '@fuzdev/gro/args.ts';
const EXIT = '@fuzdev/gro/exit.ts';

const rows = [];

// deep imports, then a shallow one
{
	const deep_1 = deep(PID);
	const shallow = [import(PID)];
	const { values, errors, engine, late } = await settle([
		['deep', deep_1],
		['shallow', shallow]
	]);
	values.push(await import(PID));
	rows.push({ case: 'deep, shallow', same: all_same(values), errors, engine, late });
}

// deep imports, a shallow one, and deep ones again
{
	const deep_1 = deep(CWD);
	const shallow = [import(CWD)];
	const deep_2 = deep(CWD);
	const { values, errors, engine, late } = await settle([
		['deep 1', deep_1],
		['shallow', shallow],
		['deep 2', deep_2]
	]);
	values.push(await import(CWD));
	rows.push({ case: 'deep, shallow, deep', same: all_same(values), errors, engine, late });
}

// deep imports twice and no shallow one
{
	const { values, errors, engine } = await settle([
		['deep 1', deep(VERSION)],
		['deep 2', deep(VERSION)]
	]);
	values.push(await import(VERSION));
	rows.push({ case: 'deep, deep', same: all_same(values), errors, engine, late: null });
}

// deep imports, then a module that imports the builtin statically, loaded in the same turn, from
// the event loop too, after the deep ones
{
	const deep_1 = deep(ENV_ERROR);
	const loaded = [import('./static_env_error.js')];
	const { values, errors, engine } = await settle([
		['deep', deep_1],
		['shallow', loaded]
	]);
	/** @type {Array<any>} */
	const namespaces = values.map((value) => {
		const { ns } = /** @type {any} */ (value);
		return ns === undefined ? value : ns;
	});
	namespaces.push(await import(ENV_ERROR));
	rows.push({
		case: 'deep, static',
		same: all_same(namespaces),
		classes: all_same(namespaces.map((namespace) => namespace.EnvError)),
		errors,
		engine,
		late: null
	});
}

// deep imports and a shallow one of a builtin whose module holds the run's handle
{
	const deep_1 = deep(ENV);
	const shallow = [import(ENV)];
	const { values, errors, engine, late } = await settle([
		['deep', deep_1],
		['shallow', shallow]
	]);
	values.push(await import(ENV));
	rows.push({
		case: 'deep, shallow, a handle',
		same: all_same(values),
		handles: all_same(values.map((namespace) => /** @type {any} */ (namespace).env)),
		errors,
		engine,
		late
	});
}

// source-phase imports, deep and shallow, which a builtin has no source for, then deep imports
// and a shallow one
{
	const sources = await settle([
		['deep', deep(ARGS, 'source')],
		['shallow', [import.source(ARGS)]]
	]);
	const deep_1 = deep(ARGS);
	const shallow = [import(ARGS)];
	const { values, errors, engine, late } = await settle([
		['deep', deep_1],
		['shallow', shallow]
	]);
	values.push(await import(ARGS));
	rows.push({
		case: 'source phase, then deep, shallow',
		sources: { imported: sources.values.length, errors: sources.errors },
		same: all_same(values),
		errors,
		engine,
		late
	});
}

// deep imports and shallow ones, interleaved
{
	/** @type {Array<[string, Array<Promise<unknown>>]>} */
	const groups = [];
	for (let i = 0; i < 3; i++) {
		groups.push([`deep ${i}`, deep(EXIT)]);
		groups.push([`shallow ${i}`, [import(EXIT)]]);
	}
	const { values, errors, engine, late } = await settle(groups);
	values.push(await import(EXIT));
	rows.push({
		case: 'deep and shallow, interleaved',
		same: all_same(values),
		errors,
		engine,
		late
	});
}

console.log(JSON.stringify(rows));
