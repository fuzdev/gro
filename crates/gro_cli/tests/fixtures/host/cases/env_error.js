import { describe, thrown } from '../describe.js';

/**
 * The shape of `value` without its `stack`, which V8 versions define differently, and whose
 * text names the files a run started from.
 * @param {object} value
 */
const describe_error = (value) => {
	const described = describe(value);
	if ('own' in described) delete described.own.stack;
	return described;
};

/**
 * What an `EnvError` is to a program: the class, an instance, its clone, a subclass, and the
 * guard, each as data.
 * @param {typeof import('../../../../../../src/lib/env_error.ts')} ns
 */
export default (ns) => {
	const { EnvError, is_env_error } = ns;
	const error = new EnvError('not_granted', 'HOME');
	const custom = new EnvError('not_granted', 'HOME', 'custom', { cause: 'why' });
	class Sub extends EnvError {
		extra = 1;
	}
	const sub = new Sub('not_granted', 'X');
	class Lookalike extends Error {}
	Object.defineProperty(Lookalike, 'name', { value: 'EnvError' });
	const hostile = {
		toString() {
			return 'hostile';
		}
	};
	const converted = new EnvError(/** @type {any} */ (hostile), /** @type {any} */ (hostile));
	const clone = structuredClone(error);
	const inherited = Object.create(EnvError.prototype);
	// what the guard runs of a caller's: the traps of a proxy on a constructor's chain or as
	// the constructor, and a getter of a constructor or of its name
	/** @type {Array<string>} */
	const ran = [];
	/** @param {string} label @param {object} target */
	const watched = (label, target) =>
		new Proxy(
			target,
			new Proxy(
				{},
				{
					get(_handler, trap) {
						ran.push(`${label} ${String(trap)}`);
						return undefined;
					}
				}
			)
		);
	const function_with_the_prototype = Object.setPrototypeOf(() => {}, EnvError.prototype);
	const named_through_a_proxy = Object.create({
		constructor: Object.create(watched('chain', { name: 'EnvError' }))
	});
	const named_by_a_proxy = Object.create({
		constructor: watched('constructor', function EnvError() {})
	});
	const constructor_by_getter = Object.create(
		Object.create(Error.prototype, {
			constructor: {
				get() {
					ran.push('constructor getter');
					return EnvError;
				}
			}
		})
	);
	const named_by_getter = Object.create({
		constructor: Object.defineProperty(function () {}, 'name', {
			get() {
				ran.push('name getter');
				return 'EnvError';
			}
		})
	});
	const longest = 'x'.repeat(2 ** 28);
	let caught;
	try {
		throw error;
	} catch (thrown_error) {
		caught = thrown_error;
	}
	return {
		namespace: describe(ns),
		class: describe(EnvError),
		class_name: EnvError.name,
		class_length: EnvError.length,
		prototype: describe(EnvError.prototype),
		extends_error: Object.getPrototypeOf(EnvError) === Error,
		prototype_extends_error: Object.getPrototypeOf(EnvError.prototype) === Error.prototype,
		constructor_is_the_class: EnvError.prototype.constructor === EnvError,
		instance: describe_error(error),
		instance_of_the_class: error instanceof EnvError,
		instance_of_error: error instanceof Error,
		is_error: /** @type {any} */ (Error).isError(error),
		tag: Object.prototype.toString.call(error),
		message: error.message,
		name: error.name,
		kind: error.kind,
		target: error.target,
		string: String(error),
		has_cause: 'cause' in error,
		stack_head: /** @type {string} */ (error.stack).split('\n')[0],
		stack_has_a_frame: /\n\s+at /.test(/** @type {string} */ (error.stack)),
		caught_is_the_error: caught === error,
		custom: describe_error(custom),
		custom_message: custom.message,
		custom_cause: custom.cause,
		null_message: new EnvError('not_granted', 'HOME', /** @type {any} */ (null)).message,
		empty_message: new EnvError('not_granted', 'HOME', '').message,
		getter_cause: new EnvError('not_granted', 'HOME', undefined, {
			get cause() {
				return 'got';
			}
		}).cause,
		no_options_cause: 'cause' in new EnvError('not_granted', 'HOME', undefined, {}),
		converted_message: converted.message,
		converted_kind_kept: converted.kind === hostile,
		message_past_the_longest_string: thrown(() => new EnvError(longest, longest)),
		symbol_kind: thrown(() => new EnvError(/** @type {any} */ (Symbol('x')), 'y')),
		symbol_message: thrown(
			() => new EnvError('not_granted', 'y', /** @type {any} */ (Symbol('x')))
		),
		clone: {
			tag: Object.prototype.toString.call(clone),
			name: clone.name,
			message: clone.message,
			has_kind: 'kind' in clone,
			is_an_error: clone instanceof Error,
			is_an_env_error: clone instanceof EnvError
		},
		json: JSON.stringify(error),
		call_without_new: thrown(() => /** @type {any} */ (EnvError)('not_granted', 'HOME')),
		subclass: {
			instance_of_sub: sub instanceof Sub,
			instance_of_the_class: sub instanceof EnvError,
			extra: sub.extra,
			kind: sub.kind,
			prototype_is_subs: Object.getPrototypeOf(sub) === Sub.prototype,
			own: describe_error(sub)
		},
		class_frozen_against_set_prototype: thrown(() =>
			Object.setPrototypeOf(EnvError, Function.prototype)
		),
		class_write: thrown(() => {
			/** @type {any} */ (EnvError).extra = 1;
		}),
		prototype_write: thrown(() => {
			/** @type {any} */ (EnvError.prototype).extra = 1;
		}),
		guard: describe(is_env_error),
		guard_name: is_env_error.name,
		guard_length: is_env_error.length,
		guard_new: thrown(() => new /** @type {any} */ (is_env_error)(error)),
		guards: {
			error: is_env_error(error),
			of_its_kind: is_env_error(error, 'not_granted'),
			of_another_kind: is_env_error(error, /** @type {any} */ ('other')),
			subclass: is_env_error(sub),
			subclass_of_its_kind: is_env_error(sub, 'not_granted'),
			lookalike: is_env_error(new Lookalike('x')),
			inherited: is_env_error(inherited),
			inherited_of_its_kind: is_env_error(inherited, 'not_granted'),
			plain_error: is_env_error(new Error('x')),
			plain_object: is_env_error({ kind: 'not_granted' }),
			null_prototype: is_env_error(Object.create(null)),
			null: is_env_error(null),
			undefined: is_env_error(undefined),
			string: is_env_error('EnvError'),
			class: is_env_error(EnvError),
			clone: is_env_error(clone),
			proxy_of_an_error: is_env_error(new Proxy(error, {})),
			function_with_the_prototype: is_env_error(function_with_the_prototype),
			named_through_a_proxy: is_env_error(named_through_a_proxy),
			named_by_a_proxy: is_env_error(named_by_a_proxy),
			constructor_by_getter: is_env_error(constructor_by_getter),
			named_by_getter: is_env_error(named_by_getter),
			ran,
			kind_by_getter: is_env_error(
				Object.create(EnvError.prototype, {
					kind: {
						get() {
							return 'not_granted';
						}
					}
				}),
				'not_granted'
			)
		}
	};
};
