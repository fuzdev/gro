import { describe, thrown } from '../describe.js';

/** What `call` throws, its class and message, or `null` when it returns. */
const refusal = (/** @type {() => unknown} */ call) => {
	try {
		call();
		return null;
	} catch (error) {
		const { constructor, message } = /** @type {any} */ (error);
		return [constructor?.name ?? typeof error, message];
	}
};

/** What `call` throws, as an env error shows it, or `null` when it returns. */
const env_refusal = (/** @type {() => unknown} */ call) => {
	try {
		call();
		return null;
	} catch (error) {
		const { constructor, message, name, kind, target } = /** @type {any} */ (error);
		return {
			class: constructor?.name,
			message,
			name,
			kind,
			target,
			is_error: error instanceof Error,
			string: String(error)
		};
	}
};

/** A record's own keys, in order, and whether it's frozen with no prototype. */
const describe_record = (/** @type {object} */ record) => ({
	keys: Reflect.ownKeys(record),
	frozen: Object.isFrozen(record),
	null_prototype: Object.getPrototypeOf(record) === null
});

/**
 * The scenarios of the env module, by name, each run with the environment the case sets, which
 * holds `GRO_A=1`, `GRO_B=two words`, and `GRO_EMPTY=` (empty) unless the case says otherwise,
 * never the developer's.
 * @type {Record<string, (ns: typeof import('../../../../../../src/lib/env.ts'), rest: Array<string>) => unknown>}
 */
const SCENARIOS = {
	// the namespace, the root handle, its prototype, and each method
	shape: (ns) => {
		const { env } = ns;
		const prototype = Object.getPrototypeOf(env);
		const narrowed = env.narrow(['GRO_A']);
		return {
			namespace: describe(ns),
			env: describe(env),
			prototype: describe(prototype),
			prototype_prototype: Object.getPrototypeOf(prototype),
			methods: ['get', 'get_all', 'narrow'].map((name) => {
				const method = prototype[name];
				return { name, shape: describe(method), length: method.length, name_of: method.name };
			}),
			narrowed: describe(narrowed),
			one_prototype: Object.getPrototypeOf(narrowed) === prototype,
			narrowed_is_another: narrowed !== env,
			json: JSON.stringify(env),
			to_string: thrown(() => `${env}`),
			thenable: 'then' in env,
			own_keys: Reflect.ownKeys(env),
			new_get: thrown(() => new /** @type {any} */ (env.get)('GRO_A')),
			property_write: thrown(() => {
				/** @type {any} */ (env).extra = 1;
			}),
			method_write: thrown(() => {
				prototype.get = () => 'forged';
			}),
			// a value one holder of a handle could leave on a method for another
			shell_write: thrown(() => {
				prototype.get.stash = 'x';
			}),
			namespace_write: thrown(() => {
				/** @type {any} */ (ns).env = null;
			}),
			still_reads: env.get('GRO_A')
		};
	},
	// what the root reads, which is every variable on Node and on an unconfined run
	read: (ns) => {
		const { env } = ns;
		const all = env.get_all();
		return {
			a: env.get('GRO_A'),
			b: env.get('GRO_B'),
			empty: env.get('GRO_EMPTY'),
			unset: env.get('GRO_UNSET'),
			inherited: env.get('toString'),
			proto: env.get('__proto__'),
			all: describe_record(all),
			all_values: { ...all },
			all_same_each_call: env.get_all() === all,
			all_write: thrown(() => {
				/** @type {any} */ (all).GRO_A = '2';
			})
		};
	},
	// each name read of the root, which a run that grants some holds only those of
	root: (ns) => {
		const { env } = ns;
		return {
			read: Object.fromEntries(
				['GRO_A', 'GRO_B', 'GRO_EMPTY', 'GRO_UNSET'].map((name) => [
					name,
					env_refusal(() => env.get(name)) ?? { value: env.get(name) ?? null }
				])
			),
			all: describe_record(env.get_all()),
			all_values: { ...env.get_all() }
		};
	},
	// handles narrowed from the root and from each other
	narrow: (ns) => {
		const { env } = ns;
		const narrowed = env.narrow(['GRO_A', 'GRO_UNSET', 'GRO_A', 'GRO_EMPTY']);
		const narrower = narrowed.narrow(['GRO_EMPTY']);
		const empty = narrowed.narrow([]);
		return {
			a: narrowed.get('GRO_A'),
			unset: narrowed.get('GRO_UNSET'),
			empty_value: narrowed.get('GRO_EMPTY'),
			all: describe_record(narrowed.get_all()),
			all_values: { ...narrowed.get_all() },
			narrower: narrower.get('GRO_EMPTY'),
			narrower_all: describe_record(narrower.get_all()),
			empty_all: describe_record(empty.get_all()),
			frozen: Object.isFrozen(narrowed) && Object.isFrozen(narrower),
			from_a_frozen_list: narrowed.narrow(Object.freeze(['GRO_A'])).get('GRO_A'),
			from_a_subclassed_array: narrowed.narrow(SubArray.of('GRO_A')).get('GRO_A')
		};
	},
	// a name outside a handle's set, read and narrowed to
	not_granted: (ns) => {
		const { env } = ns;
		const narrowed = env.narrow(['GRO_A']);
		return {
			get: env_refusal(() => narrowed.get('GRO_B')),
			get_unset: env_refusal(() => narrowed.get('GRO_UNSET')),
			narrow: env_refusal(() => narrowed.narrow(['GRO_A', 'GRO_B'])),
			narrower: env_refusal(() => narrowed.narrow(['GRO_A']).narrow(['GRO_EMPTY'])),
			// the first name's fault, in order
			first_fault: refusal(() => narrowed.narrow(['GRO_B', 1])),
			first_type_fault: refusal(() => narrowed.narrow([1, 'GRO_B']))
		};
	},
	// names and lists that are none, each a `TypeError`, its message naming no value
	invalid: (ns) => {
		const { env } = ns;
		/** @type {Array<[string, unknown]>} */
		const names = [
			['a number', 1],
			['undefined', undefined],
			['null', null],
			['a boxed string', new String('GRO_A')],
			['a symbol', Symbol('GRO_A')],
			['empty', ''],
			['a NUL', 'GRO\0A'],
			['a lone surrogate', 'GRO_\uD800'],
			['a =', 'GRO_A=secret'],
			['a = first', '=GRO_A']
		];
		/** @type {Array<[string, unknown]>} */
		const lists = [
			['a string', 'GRO_A'],
			['undefined', undefined],
			['an array-like', { length: 1, 0: 'GRO_A' }],
			['a set', new Set(['GRO_A'])],
			['a hole', [, 'GRO_A']],
			['a glob', ['GRO_*']],
			['a number', ['GRO_A', 1]],
			['empty', ['']],
			['a =', ['GRO_A=secret']],
			['a NUL', ['GRO\0A']],
			['a lone surrogate', ['\uDC00']]
		];
		return {
			get: names.map(([what, name]) => [what, refusal(() => env.get(/** @type {any} */ (name)))]),
			get_glob: env.get('GRO_*'),
			narrow: lists.map(([what, list]) => [
				what,
				refusal(() => env.narrow(/** @type {any} */ (list)))
			])
		};
	},
	// a method called on what isn't a handle
	receiver: (ns) => {
		const { env } = ns;
		const prototype = Object.getPrototypeOf(env);
		const look_alike = Object.create(prototype);
		const proxy = new Proxy(env, {});
		/** @type {Array<[string, unknown]>} */
		const receivers = [
			['undefined', undefined],
			['an object', {}],
			['a look-alike', look_alike],
			['a frozen look-alike', Object.freeze(Object.create(prototype))],
			['a proxy', proxy],
			['the prototype', prototype]
		];
		return receivers.map(([what, receiver]) => [
			what,
			['get', 'get_all', 'narrow'].map((name) => {
				const args = name === 'narrow' ? [['GRO_A']] : name === 'get' ? ['GRO_A'] : [];
				return [name, thrown(() => Reflect.apply(prototype[name], receiver, args))];
			})
		]);
	},
	// what an op runs no code of: a name list given through a getter or a proxy
	given_through_code: (ns) => {
		const { env } = ns;
		const by_getter = ['GRO_A'];
		Object.defineProperty(by_getter, 0, { get: () => 'GRO_A' });
		return {
			getter: refusal(() => env.narrow(by_getter).get('GRO_A')),
			proxy: refusal(() => env.narrow(new Proxy(['GRO_A'], {})).get('GRO_A'))
		};
	}
};

class SubArray extends Array {}

/**
 * @param {typeof import('../../../../../../src/lib/env.ts')} ns
 * @param {Array<string>} rest - the scenario's name
 */
export default (ns, [name]) => {
	const scenario = SCENARIOS[/** @type {string} */ (name)];
	if (scenario === undefined) throw new Error(`no scenario ${name}`);
	return scenario(ns, []);
};
