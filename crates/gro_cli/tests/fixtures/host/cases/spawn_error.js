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
 * An error's message and whether it has a `cause`.
 * @param {Error} error
 */
const message_and_cause = (error) => ({ message: error.message, has_cause: 'cause' in error });

/**
 * What a `SpawnError` is to a program: the class, an instance, its clone, a subclass, and the
 * guard, each as data.
 * @param {typeof import('../../../../../../src/lib/spawn_error.ts')} ns
 */
export default (ns) => {
	const { SpawnError, is_spawn_error } = ns;
	const error = new SpawnError('not_found', 'git');
	const custom = new SpawnError('not_found', 'git', { cause: 'why' });
	class Sub extends SpawnError {
		extra = 1;
	}
	const sub = new Sub('not_found', 'x');
	class Lookalike extends Error {}
	Object.defineProperty(Lookalike, 'name', { value: 'SpawnError' });
	const hostile = {
		toString() {
			return 'hostile';
		}
	};
	const converted = new SpawnError(/** @type {any} */ (hostile), /** @type {any} */ (hostile));
	const clone = structuredClone(error);
	const inherited = Object.create(SpawnError.prototype);
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
	const function_with_the_prototype = Object.setPrototypeOf(() => {}, SpawnError.prototype);
	const named_through_a_proxy = Object.create({
		constructor: Object.create(watched('chain', { name: 'SpawnError' }))
	});
	const named_by_a_proxy = Object.create({
		constructor: watched('constructor', function SpawnError() {})
	});
	const constructor_by_getter = Object.create(
		Object.create(Error.prototype, {
			constructor: {
				get() {
					ran.push('constructor getter');
					return SpawnError;
				}
			}
		})
	);
	const named_by_getter = Object.create({
		constructor: Object.defineProperty(function () {}, 'name', {
			get() {
				ran.push('name getter');
				return 'SpawnError';
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
		class: describe(SpawnError),
		class_name: SpawnError.name,
		class_length: SpawnError.length,
		prototype: describe(SpawnError.prototype),
		extends_error: Object.getPrototypeOf(SpawnError) === Error,
		prototype_extends_error: Object.getPrototypeOf(SpawnError.prototype) === Error.prototype,
		constructor_is_the_class: SpawnError.prototype.constructor === SpawnError,
		instance: describe_error(error),
		instance_of_the_class: error instanceof SpawnError,
		instance_of_error: error instanceof Error,
		is_error: /** @type {any} */ (Error).isError(error),
		tag: Object.prototype.toString.call(error),
		message: error.message,
		name: error.name,
		kind: error.kind,
		program: error.program,
		string: String(error),
		has_cause: 'cause' in error,
		stack_head: /** @type {string} */ (error.stack).split('\n')[0],
		stack_has_a_frame: /\n\s+at /.test(/** @type {string} */ (error.stack)),
		caught_is_the_error: caught === error,
		custom: describe_error(custom),
		custom_message: custom.message,
		custom_cause: custom.cause,
		// a third argument is the options, never a message
		string_options: message_and_cause(
			new SpawnError('not_found', 'git', /** @type {any} */ ('custom'))
		),
		null_options: message_and_cause(new SpawnError('not_found', 'git', /** @type {any} */ (null))),
		getter_cause: new SpawnError('not_found', 'git', {
			get cause() {
				return 'got';
			}
		}).cause,
		no_options_cause: 'cause' in new SpawnError('not_found', 'git', {}),
		converted_message: converted.message,
		converted_kind_kept: converted.kind === hostile,
		message_past_the_longest_string: thrown(() => new SpawnError(longest, longest)),
		symbol_kind: thrown(() => new SpawnError(/** @type {any} */ (Symbol('x')), 'y')),
		symbol_target: thrown(() => new SpawnError('not_found', /** @type {any} */ (Symbol('x')))),
		symbol_options: message_and_cause(
			new SpawnError('not_found', 'y', /** @type {any} */ (Symbol('x')))
		),
		clone: {
			tag: Object.prototype.toString.call(clone),
			name: clone.name,
			message: clone.message,
			has_kind: 'kind' in clone,
			is_an_error: clone instanceof Error,
			is_an_env_error: clone instanceof SpawnError
		},
		json: JSON.stringify(error),
		call_without_new: thrown(() => /** @type {any} */ (SpawnError)('not_found', 'git')),
		subclass: {
			instance_of_sub: sub instanceof Sub,
			instance_of_the_class: sub instanceof SpawnError,
			extra: sub.extra,
			kind: sub.kind,
			prototype_is_subs: Object.getPrototypeOf(sub) === Sub.prototype,
			own: describe_error(sub)
		},
		class_frozen_against_set_prototype: thrown(() =>
			Object.setPrototypeOf(SpawnError, Function.prototype)
		),
		class_write: thrown(() => {
			/** @type {any} */ (SpawnError).extra = 1;
		}),
		prototype_write: thrown(() => {
			/** @type {any} */ (SpawnError.prototype).extra = 1;
		}),
		guard: describe(is_spawn_error),
		guard_name: is_spawn_error.name,
		guard_length: is_spawn_error.length,
		guard_new: thrown(() => new /** @type {any} */ (is_spawn_error)(error)),
		guards: {
			error: is_spawn_error(error),
			of_its_kind: is_spawn_error(error, 'not_found'),
			of_another_kind: is_spawn_error(error, /** @type {any} */ ('other')),
			subclass: is_spawn_error(sub),
			subclass_of_its_kind: is_spawn_error(sub, 'not_found'),
			lookalike: is_spawn_error(new Lookalike('x')),
			inherited: is_spawn_error(inherited),
			inherited_of_its_kind: is_spawn_error(inherited, 'not_found'),
			plain_error: is_spawn_error(new Error('x')),
			plain_object: is_spawn_error({ kind: 'not_found' }),
			null_prototype: is_spawn_error(Object.create(null)),
			null: is_spawn_error(null),
			undefined: is_spawn_error(undefined),
			string: is_spawn_error('SpawnError'),
			class: is_spawn_error(SpawnError),
			clone: is_spawn_error(clone),
			proxy_of_an_error: is_spawn_error(new Proxy(error, {})),
			function_with_the_prototype: is_spawn_error(function_with_the_prototype),
			named_through_a_proxy: is_spawn_error(named_through_a_proxy),
			named_by_a_proxy: is_spawn_error(named_by_a_proxy),
			constructor_by_getter: is_spawn_error(constructor_by_getter),
			named_by_getter: is_spawn_error(named_by_getter),
			ran,
			kind_by_getter: is_spawn_error(
				Object.create(SpawnError.prototype, {
					kind: {
						get() {
							return 'not_found';
						}
					}
				}),
				'not_found'
			)
		}
	};
};
