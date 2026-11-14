// A first import made to fail. `import()` of a builtin no module has imported yet runs the
// builtin's implementation on the caller's stack, so a caller with almost none left makes the
// implementation overflow partway, at each of its calls in turn as the caller leaves it more.
// For every builtin (`modules.js`, which the test writes from the builtin table), this starts
// the import from ever higher above the stack's limit until one succeeds, and prints what the
// failed ones rejected with, the places in the implementation they were cut short at (the
// first frame a failure names, when it's the implementation's, as `line:column`: the import
// ran it, and didn't fail before), the exports of the one that succeeded, and whether a later
// import is that same module.

import { SPECIFIERS } from './modules.js';

/**
 * Starts `import(specifier)` from `up` frames above the deepest frame the stack allows.
 * @param {string} specifier
 * @param {number} up
 * @returns {Promise<any> | undefined} the import's promise, when the stack let it start
 */
const import_near_the_limit = (specifier, up) => {
	/** @type {Promise<any> | undefined} */
	let started;
	const dive = () => {
		try {
			dive();
		} catch (error) {
			if (up-- === 0) started = import(specifier);
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

/** How far above the limit an import is started before the test gives up on it. */
const HIGHEST = 20000;

const rows = [];
for (const specifier of SPECIFIERS) {
	/** @type {Array<string>} */
	const rejections = [];
	/** @type {Array<string>} */
	const cut_at = [];
	const frame = `    at ext:gro/builtin/${specifier}:`;
	let whole;
	// coarse steps to the first import that succeeds are enough to cut it short many ways
	for (let up = 0; up < HIGHEST && whole === undefined; up += 3) {
		const started = import_near_the_limit(specifier, up);
		if (started === undefined) continue;
		try {
			whole = await started;
		} catch (error) {
			const message = String(/** @type {Error} */ (error).message);
			const [report = '', first = ''] = message.split('\n');
			if (!rejections.includes(report)) rejections.push(report);
			if (first.startsWith(frame)) {
				const place = first.slice(frame.length);
				if (!cut_at.includes(place)) cut_at.push(place);
			}
		}
	}
	const again = await import(specifier);
	rows.push({
		specifier,
		rejections,
		cut_at,
		exports: whole === undefined ? null : Object.keys(whole).sort(),
		same: again === whole
	});
}
console.log(JSON.stringify(rows));
