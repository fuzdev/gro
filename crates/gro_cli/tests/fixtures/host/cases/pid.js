import { describe, thrown } from '../describe.js';

/**
 * @param {typeof import('../../../../../../src/lib/pid.ts')} ns
 */
export default (ns) => ({
	namespace: describe(ns),
	type: typeof ns.pid,
	positive_integer: Number.isSafeInteger(ns.pid) && ns.pid > 0,
	namespace_write_throws: thrown(() => {
		/** @type {any} */ (ns).pid = 1;
	})
});
