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
 * What a `NetError` is to a program: the class, an instance, its clone, a subclass, and the
 * guard, each as data.
 * @param {typeof import('../../../../../../src/lib/net_error.ts')} ns
 */
export default (ns) => {
	const { NetError, is_net_error } = ns;
	const error = new NetError('not_granted', 'example.com:443');
	const custom = new NetError('not_granted', 'example.com:443', { cause: 'why' });
	class Sub extends NetError {
		extra = 1;
	}
	const sub = new Sub('not_granted', 'X');
	class Lookalike extends Error {}
	Object.defineProperty(Lookalike, 'name', { value: 'NetError' });
	const hostile = {
		toString() {
			return 'hostile';
		}
	};
	const converted = new NetError(/** @type {any} */ (hostile), /** @type {any} */ (hostile));
	const clone = structuredClone(error);
	const inherited = Object.create(NetError.prototype);
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
	const function_with_the_prototype = Object.setPrototypeOf(() => {}, NetError.prototype);
	const named_through_a_proxy = Object.create({
		constructor: Object.create(watched('chain', { name: 'NetError' }))
	});
	const named_by_a_proxy = Object.create({
		constructor: watched('constructor', function NetError() {})
	});
	const constructor_by_getter = Object.create(
		Object.create(Error.prototype, {
			constructor: {
				get() {
					ran.push('constructor getter');
					return NetError;
				}
			}
		})
	);
	const named_by_getter = Object.create({
		constructor: Object.defineProperty(function () {}, 'name', {
			get() {
				ran.push('name getter');
				return 'NetError';
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
		class: describe(NetError),
		class_name: NetError.name,
		class_length: NetError.length,
		prototype: describe(NetError.prototype),
		extends_error: Object.getPrototypeOf(NetError) === Error,
		prototype_extends_error: Object.getPrototypeOf(NetError.prototype) === Error.prototype,
		constructor_is_the_class: NetError.prototype.constructor === NetError,
		instance: describe_error(error),
		instance_of_the_class: error instanceof NetError,
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
			new NetError('not_granted', 'example.com:443', /** @type {any} */ ('custom'))
		),
		null_options: message_and_cause(
			new NetError('not_granted', 'example.com:443', /** @type {any} */ (null))
		),
		getter_cause: new NetError('not_granted', 'example.com:443', {
			get cause() {
				return 'got';
			}
		}).cause,
		no_options_cause: 'cause' in new NetError('not_granted', 'example.com:443', {}),
		converted_message: converted.message,
		converted_kind_kept: converted.kind === hostile,
		message_past_the_longest_string: thrown(() => new NetError(longest, longest)),
		symbol_kind: thrown(() => new NetError(/** @type {any} */ (Symbol('x')), 'y')),
		symbol_target: thrown(() => new NetError('not_granted', /** @type {any} */ (Symbol('x')))),
		symbol_options: message_and_cause(
			new NetError('not_granted', 'y', /** @type {any} */ (Symbol('x')))
		),
		clone: {
			tag: Object.prototype.toString.call(clone),
			name: clone.name,
			message: clone.message,
			has_kind: 'kind' in clone,
			is_an_error: clone instanceof Error,
			is_a_net_error: clone instanceof NetError
		},
		json: JSON.stringify(error),
		call_without_new: thrown(() => /** @type {any} */ (NetError)('not_granted', 'example.com:443')),
		subclass: {
			instance_of_sub: sub instanceof Sub,
			instance_of_the_class: sub instanceof NetError,
			extra: sub.extra,
			kind: sub.kind,
			prototype_is_subs: Object.getPrototypeOf(sub) === Sub.prototype,
			own: describe_error(sub)
		},
		class_frozen_against_set_prototype: thrown(() =>
			Object.setPrototypeOf(NetError, Function.prototype)
		),
		class_write: thrown(() => {
			/** @type {any} */ (NetError).extra = 1;
		}),
		prototype_write: thrown(() => {
			/** @type {any} */ (NetError.prototype).extra = 1;
		}),
		guard: describe(is_net_error),
		guard_name: is_net_error.name,
		guard_length: is_net_error.length,
		guard_new: thrown(() => new /** @type {any} */ (is_net_error)(error)),
		guards: {
			error: is_net_error(error),
			of_its_kind: is_net_error(error, 'not_granted'),
			of_another_kind: is_net_error(error, /** @type {any} */ ('other')),
			subclass: is_net_error(sub),
			subclass_of_its_kind: is_net_error(sub, 'not_granted'),
			lookalike: is_net_error(new Lookalike('x')),
			inherited: is_net_error(inherited),
			inherited_of_its_kind: is_net_error(inherited, 'not_granted'),
			plain_error: is_net_error(new Error('x')),
			plain_object: is_net_error({ kind: 'not_granted' }),
			null_prototype: is_net_error(Object.create(null)),
			null: is_net_error(null),
			undefined: is_net_error(undefined),
			string: is_net_error('NetError'),
			class: is_net_error(NetError),
			clone: is_net_error(clone),
			proxy_of_an_error: is_net_error(new Proxy(error, {})),
			function_with_the_prototype: is_net_error(function_with_the_prototype),
			named_through_a_proxy: is_net_error(named_through_a_proxy),
			named_by_a_proxy: is_net_error(named_by_a_proxy),
			constructor_by_getter: is_net_error(constructor_by_getter),
			named_by_getter: is_net_error(named_by_getter),
			ran,
			kind_by_getter: is_net_error(
				Object.create(NetError.prototype, {
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
