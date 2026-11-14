import { describe, thrown } from '../describe.js';

/**
 * @param {typeof import('../../../../../../src/lib/args.ts')} ns
 * @param {Array<string>} rest - the arguments after the case's name
 */
export default (ns, rest) => ({
	namespace: describe(ns),
	args: ns.args,
	are_the_arguments: JSON.stringify(ns.args) === JSON.stringify(['args', ...rest]),
	shape: describe(ns.args),
	is_array: Array.isArray(ns.args),
	element_write_throws: thrown(() => {
		/** @type {Array<string>} */ (ns.args)[0] = 'z';
	}),
	push_throws: thrown(() => /** @type {Array<string>} */ (ns.args).push('z')),
	namespace_write_throws: thrown(() => {
		/** @type {any} */ (ns).args = [];
	})
});
