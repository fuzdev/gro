import { describe, thrown } from '../describe.js';

/**
 * @param {typeof import('../../../../../../src/lib/runtime_version.ts')} ns
 */
export default (ns) => ({
	namespace: describe(ns),
	type: typeof ns.runtime_version,
	name_and_version: /^[a-z]+\/[^\s/]+$/.test(ns.runtime_version),
	runtime: ns.runtime_version.split('/')[0],
	namespace_write_throws: thrown(() => {
		/** @type {any} */ (ns).runtime_version = '';
	})
});
