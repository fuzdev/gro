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
 * What a `StdioError` is to a program: the class, an instance, its clone, a subclass, and the
 * guard, each as data.
 * @param {typeof import('../../../../../../src/lib/stdio_error.ts')} ns
 */
export default (ns) => {
	const { StdioError, is_stdio_error } = ns;
	const error = new StdioError('io_error', 'stdout');
	const custom = new StdioError('io_error', 'stdout', { cause: 'why' });
	class Sub extends StdioError {
		extra = 1;
	}
	const sub = new Sub('io_error', 'X');
	class Lookalike extends Error {}
	Object.defineProperty(Lookalike, 'name', { value: 'StdioError' });
	const hostile = {
		toString() {
			return 'hostile';
		}
	};
	const converted = new StdioError(/** @type {any} */ (hostile), /** @type {any} */ (hostile));
	const clone = structuredClone(error);
	const inherited = Object.create(StdioError.prototype);
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
	const function_with_the_prototype = Object.setPrototypeOf(() => {}, StdioError.prototype);
	const named_through_a_proxy = Object.create({
		constructor: Object.create(watched('chain', { name: 'StdioError' }))
	});
	const named_by_a_proxy = Object.create({
		constructor: watched('constructor', function StdioError() {})
	});
	const constructor_by_getter = Object.create(
		Object.create(Error.prototype, {
			constructor: {
				get() {
					ran.push('constructor getter');
					return StdioError;
				}
			}
		})
	);
	const named_by_getter = Object.create({
		constructor: Object.defineProperty(function () {}, 'name', {
			get() {
				ran.push('name getter');
				return 'StdioError';
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
		class: describe(StdioError),
		class_name: StdioError.name,
		class_length: StdioError.length,
		prototype: describe(StdioError.prototype),
		extends_error: Object.getPrototypeOf(StdioError) === Error,
		prototype_extends_error: Object.getPrototypeOf(StdioError.prototype) === Error.prototype,
		constructor_is_the_class: StdioError.prototype.constructor === StdioError,
		instance: describe_error(error),
		instance_of_the_class: error instanceof StdioError,
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
		// a third argument is the options, never a message
		string_options: message_and_cause(
			new StdioError('io_error', 'stdout', /** @type {any} */ ('custom'))
		),
		null_options: message_and_cause(
			new StdioError('io_error', 'stdout', /** @type {any} */ (null))
		),
		getter_cause: new StdioError('io_error', 'stdout', {
			get cause() {
				return 'got';
			}
		}).cause,
		no_options_cause: 'cause' in new StdioError('io_error', 'stdout', {}),
		converted_message: converted.message,
		converted_kind_kept: converted.kind === hostile,
		message_past_the_longest_string: thrown(() => new StdioError(longest, longest)),
		symbol_kind: thrown(() => new StdioError(/** @type {any} */ (Symbol('x')), 'y')),
		symbol_target: thrown(() => new StdioError('io_error', /** @type {any} */ (Symbol('x')))),
		symbol_options: message_and_cause(
			new StdioError('io_error', 'y', /** @type {any} */ (Symbol('x')))
		),
		clone: {
			tag: Object.prototype.toString.call(clone),
			name: clone.name,
			message: clone.message,
			has_kind: 'kind' in clone,
			is_an_error: clone instanceof Error,
			is_an_stdio_error: clone instanceof StdioError
		},
		json: JSON.stringify(error),
		call_without_new: thrown(() => /** @type {any} */ (StdioError)('io_error', 'stdout')),
		subclass: {
			instance_of_sub: sub instanceof Sub,
			instance_of_the_class: sub instanceof StdioError,
			extra: sub.extra,
			kind: sub.kind,
			prototype_is_subs: Object.getPrototypeOf(sub) === Sub.prototype,
			own: describe_error(sub)
		},
		class_frozen_against_set_prototype: thrown(() =>
			Object.setPrototypeOf(StdioError, Function.prototype)
		),
		class_write: thrown(() => {
			/** @type {any} */ (StdioError).extra = 1;
		}),
		prototype_write: thrown(() => {
			/** @type {any} */ (StdioError.prototype).extra = 1;
		}),
		guard: describe(is_stdio_error),
		guard_name: is_stdio_error.name,
		guard_length: is_stdio_error.length,
		guard_new: thrown(() => new /** @type {any} */ (is_stdio_error)(error)),
		guards: {
			error: is_stdio_error(error),
			of_its_kind: is_stdio_error(error, 'io_error'),
			of_another_kind: is_stdio_error(error, /** @type {any} */ ('other')),
			subclass: is_stdio_error(sub),
			subclass_of_its_kind: is_stdio_error(sub, 'io_error'),
			lookalike: is_stdio_error(new Lookalike('x')),
			inherited: is_stdio_error(inherited),
			inherited_of_its_kind: is_stdio_error(inherited, 'io_error'),
			plain_error: is_stdio_error(new Error('x')),
			plain_object: is_stdio_error({ kind: 'io_error' }),
			null_prototype: is_stdio_error(Object.create(null)),
			null: is_stdio_error(null),
			undefined: is_stdio_error(undefined),
			string: is_stdio_error('StdioError'),
			class: is_stdio_error(StdioError),
			clone: is_stdio_error(clone),
			proxy_of_an_error: is_stdio_error(new Proxy(error, {})),
			function_with_the_prototype: is_stdio_error(function_with_the_prototype),
			named_through_a_proxy: is_stdio_error(named_through_a_proxy),
			named_by_a_proxy: is_stdio_error(named_by_a_proxy),
			constructor_by_getter: is_stdio_error(constructor_by_getter),
			named_by_getter: is_stdio_error(named_by_getter),
			ran,
			kind_by_getter: is_stdio_error(
				Object.create(StdioError.prototype, {
					kind: {
						get() {
							return 'io_error';
						}
					}
				}),
				'io_error'
			)
		}
	};
};
