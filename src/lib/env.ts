/**
 * The env host module: the environment of the running process, as an import rather than a global.
 *
 * An `Env` is read-only and holds a snapshot taken when this module loads, the runtime's
 * fixed environment: a write to the global's `process.env` afterward changes nothing here,
 * and only names listed by `env.narrow` reach a narrowed handle. A name outside its set throws
 * `not_granted`, so a denial is never read as a variable that's unset, which is
 * `undefined` from `get`. Library edges take plain records, built at a root from `get_all`.
 * An environment value can be a secret: this module prints none, its errors naming the variable,
 * never the value, so what a value becomes after a read is up to the caller.
 *
 * Handles and their prototypes are frozen, null-prototype so none of them is thenable even with a
 * polluted `Object.prototype.then`, and constructing one takes a key only this module holds.
 * A method called on any other object throws `TypeError`, private fields being the only state.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can read `process.env`,
 *   so a narrowed `Env` bounds only code that uses it, a review signal, not a boundary.
 * - The root `Env` allows every name, where the runtime's holds only the granted ones and
 *   a name outside them throws `not_granted`.
 * - The snapshot is taken at this module's load, so a write to `process.env` before that shows.
 *
 * @module
 */

import { EnvError } from './env_error.ts';
import { check_env_name } from './internal/env_name.ts';
import { create_record } from './internal/records.ts';

const host_env = globalThis.process.env;
const { isArray } = Array;
const { freeze, keys, setPrototypeOf } = Object;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('env handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('env handles come from `env`, not a constructor');
	}
};

type Allowed = Readonly<Record<string, true | undefined>>;

/**
 * The environment, read-only, a snapshot taken when the module loads.
 */
export interface Env {
	/**
	 * A variable's value, `undefined` when it's unset. Own names only, so `toString` is unset.
	 * @throws `TypeError` - for a name that isn't a non-empty, well-formed string without NUL or `=`
	 * @throws `EnvError` - `not_granted`, a name outside this handle's set
	 */
	get(name: string): string | undefined;
	/**
	 * Every variable this handle allows that's set, a frozen null-prototype record.
	 * Pass it, or part of it, where a library takes a plain record.
	 */
	get_all(): Readonly<Record<string, string>>;
	/**
	 * A handle limited to `names`, each an exact, case-sensitive name, never a glob, so a holder
	 * of the handle can't enumerate more than it's given. A name that's unset stays
	 * in the set, as `undefined` from `get`. It can only shrink.
	 * @throws `TypeError` - for a name `get` refuses, or one holding `*`
	 * @throws `EnvError` - `not_granted`, a name outside this handle's set
	 */
	narrow(names: ReadonlyArray<string>): Env;
}

/**
 * A narrowed set's name: exact, with no `*` a reader could take for a glob.
 */
const check_narrow_name = (value: unknown): string => {
	const name = check_env_name(value, 'a `narrow` name');
	if (name.includes('*')) {
		throw new TypeError(`a \`narrow\` name is exact, never a glob: ${name}`);
	}
	return name;
};

/**
 * The environment as a frozen null-prototype record, own properties only, so nothing inherited
 * from `Object.prototype` ever reads as a variable.
 */
const to_env_snapshot = (): Readonly<Record<string, string>> => {
	const snapshot = create_record<Record<string, string>>({});
	const names = keys(host_env);
	for (let i = 0; i < names.length; i++) {
		const name = names[i]!;
		const value = host_env[name];
		if (typeof value === 'string') snapshot[name] = value;
	}
	return freeze(snapshot);
};

class NodeEnv implements Env {
	readonly #vars: Readonly<Record<string, string>>;
	readonly #allowed: Allowed | null;

	constructor(key: symbol, vars: Readonly<Record<string, string>>, allowed: Allowed | null) {
		check_handle_key(key);
		this.#vars = vars;
		this.#allowed = allowed;
	}

	get(name: string): string | undefined {
		const vars = this.#vars;
		const allowed = this.#allowed;
		const checked = check_env_name(name, 'an environment name');
		if (allowed !== null && allowed[checked] !== true) {
			throw new EnvError('not_granted', checked, `not_granted: ${checked} isn't in this env's set`);
		}
		return vars[checked];
	}

	get_all(): Readonly<Record<string, string>> {
		return this.#vars;
	}

	narrow(names: ReadonlyArray<string>): Env {
		const parent_vars = this.#vars;
		const parent_allowed = this.#allowed;
		if (!isArray(names)) throw new TypeError('env.narrow takes an array of names');
		const vars = create_record<Record<string, string>>({});
		const allowed = create_record<Record<string, true>>({});
		const length = names.length;
		for (let i = 0; i < length; i++) {
			const name = check_narrow_name(names[i]);
			if (parent_allowed !== null && parent_allowed[name] !== true) {
				throw new EnvError(
					'not_granted',
					name,
					`not_granted: ${name} isn't in this env's set, and narrow only shrinks`
				);
			}
			allowed[name] = true;
			const value = parent_vars[name];
			if (value !== undefined) vars[name] = value;
		}
		return freeze(new NodeEnv(HANDLE_KEY, freeze(vars), freeze(allowed)));
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
setPrototypeOf(NodeEnv.prototype, null);
freeze(NodeEnv.prototype);
freeze(NodeEnv);

/**
 * The env host module's root handle, the environment when the module loaded. Import it where
 * authority enters (a task, a script, an entry module) and hand a narrower `env.narrow(...)`
 * or plain records down.
 */
export const env: Env = freeze(new NodeEnv(HANDLE_KEY, to_env_snapshot(), null));
