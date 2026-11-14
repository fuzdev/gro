import { describe, thrown } from '../describe.js';

/**
 * @param {typeof import('../../../../../../src/lib/cwd.ts')} ns
 * @param {Array<string>} rest - the directory the case was started in
 */
export default (ns, [started_in]) => {
	const { cwd } = ns;
	return {
		namespace: describe(ns),
		shape: describe(cwd),
		name: cwd.name,
		length: cwd.length,
		is_the_start_directory: cwd() === started_in,
		returns: typeof cwd(),
		same_each_call: cwd() === cwd(),
		borrowed_receiver: cwd.call({}) === started_in,
		new_throws: thrown(() => new /** @type {any} */ (cwd)()),
		property_write_throws: thrown(() => {
			/** @type {any} */ (cwd).extra = 1;
		}),
		namespace_write_throws: thrown(() => {
			/** @type {any} */ (ns).cwd = null;
		})
	};
};
