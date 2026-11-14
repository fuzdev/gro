import { describe, test, assert, beforeEach, vi } from 'vitest';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { env, type Env } from '#lib/env.ts';
import { is_env_error, type EnvError, type EnvErrorKind } from '#lib/env_error.ts';
import { to_check_handles_body, to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const MODULE_URL = to_lib_url('env.ts');

const runner = use_fixture_runner(`import {env} from ${JSON.stringify(MODULE_URL)};`);
const { run_node, run_fixture_json } = runner;

const assert_env_error = (fn: () => unknown, kind: EnvErrorKind): EnvError => {
	try {
		fn();
	} catch (error) {
		assert.ok(is_env_error(error, kind), `expected ${kind}, got ${String(error)}`);
		return error;
	}
	throw new Error(`expected an EnvError of kind ${kind}`);
};

describe('env', () => {
	test('reads a variable by name, an unset one as undefined', () => {
		const result = run_fixture_json(
			`console.log(JSON.stringify([env.get('GRO_FAKE_A'), env.get('GRO_FAKE_EMPTY'), env.get('GRO_FAKE_UNSET') === undefined]));`,
			{ env: { GRO_FAKE_A: 'fake-a', GRO_FAKE_EMPTY: '' } }
		);
		assert.deepEqual(result, ['fake-a', '', true]);
	});

	test('`get_all` is a frozen null-prototype record of the variables', () => {
		const result = run_fixture_json(
			`const all = env.get_all();
			 console.log(JSON.stringify({
				proto: Object.getPrototypeOf(all) === null,
				frozen: Object.isFrozen(all),
				a: all.GRO_FAKE_A,
				keys: Object.keys(all),
			 }));`,
			{ env: { GRO_FAKE_A: 'fake-a' } }
		);
		assert.deepEqual(result, { proto: true, frozen: true, a: 'fake-a', keys: ['GRO_FAKE_A'] });
	});

	test('an inherited name is no variable, even with a polluted prototype', () => {
		const result = run_fixture_json(
			`Object.prototype.GRO_FAKE_POLLUTED = 'polluted';
			 const narrowed = env.narrow(['toString', 'GRO_FAKE_POLLUTED']);
			 console.log(JSON.stringify([
				typeof env.get('toString'),
				typeof env.get('hasOwnProperty'),
				typeof env.get('__proto__'),
				typeof env.get('GRO_FAKE_POLLUTED'),
				typeof narrowed.get('toString'),
				typeof narrowed.get('GRO_FAKE_POLLUTED'),
				'GRO_FAKE_POLLUTED' in env.get_all()
			 ]));`
		);
		assert.deepEqual(result, [
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			false
		]);
	});

	test('the environment is a snapshot taken when the module loads', () => {
		const result = run_fixture_json(
			`globalThis.process.env.GRO_FAKE_LATE = 'late';
			 delete globalThis.process.env.GRO_FAKE_A;
			 console.log(JSON.stringify([env.get('GRO_FAKE_A'), env.get('GRO_FAKE_LATE') === undefined]));`,
			{ env: { GRO_FAKE_A: 'fake-a' } }
		);
		assert.deepEqual(result, ['fake-a', true]);
	});

	test('a write before the module loads is in the snapshot', () => {
		const path = join(runner.sandbox.base, 'early.ts');
		writeFileSync(
			path,
			`globalThis.process.env.GRO_FAKE_EARLY = 'early';
			 const {env} = await import(${JSON.stringify(MODULE_URL)});
			 console.log(JSON.stringify(env.get('GRO_FAKE_EARLY')));`
		);
		const { status, stdout, stderr } = run_node([path]);
		assert.strictEqual(status, 0, stderr);
		assert.deepEqual(JSON.parse(stdout), 'early');
	});

	test('refuses a malformed name as a TypeError, not a missing variable', () => {
		for (const bad of ['', 'A=B', 'A\0B', '\ud800', 1, null, undefined, {}]) {
			assert.throws(() => env.get(bad as string), TypeError);
		}
	});

	test('a refused name with a `=` does not echo what could be a value', () => {
		for (const fn of [
			() => env.get('GRO_FAKE=fake-secret'),
			() => env.narrow(['GRO_FAKE=fake-secret'])
		]) {
			assert.throws(fn, TypeError);
			try {
				fn();
			} catch (error) {
				assert.notInclude((error as Error).message, 'fake-secret');
			}
		}
	});

	describe('narrow', () => {
		let narrowed: Env;

		beforeEach(() => {
			narrowed = env.narrow(['GRO_FAKE_A', 'GRO_FAKE_B']);
		});

		test('a name outside the set throws `not_granted`, it does not read as unset', () => {
			const error = assert_env_error(() => narrowed.get('GRO_FAKE_C'), 'not_granted');
			assert.strictEqual(error.target, 'GRO_FAKE_C');
			assert.notInclude(error.message, 'fake');
		});

		test('a name in the set that is unset reads as undefined', () => {
			assert.strictEqual(narrowed.get('GRO_FAKE_A'), undefined);
		});

		test('reads through only the set, from the parent snapshot', () => {
			const result = run_fixture_json(
				`const narrowed = env.narrow(['GRO_FAKE_A', 'GRO_FAKE_UNSET']);
				 let refused = false;
				 try { narrowed.get('GRO_FAKE_B'); } catch (e) { refused = e.kind === 'not_granted'; }
				 console.log(JSON.stringify({
					a: narrowed.get('GRO_FAKE_A'),
					unset: narrowed.get('GRO_FAKE_UNSET') === undefined,
					refused,
					all: Object.keys(narrowed.get_all()),
					null_proto: Object.getPrototypeOf(narrowed.get_all()) === null
				 }));`,
				{ env: { GRO_FAKE_A: 'fake-a', GRO_FAKE_B: 'fake-b' } }
			);
			assert.deepEqual(result, {
				a: 'fake-a',
				unset: true,
				refused: true,
				all: ['GRO_FAKE_A'],
				null_proto: true
			});
		});

		test('only shrinks, as a name outside the parent is `not_granted`', () => {
			narrowed.narrow(['GRO_FAKE_A']);
			assert_env_error(() => narrowed.narrow(['GRO_FAKE_C']), 'not_granted');
		});

		test('takes exact names only, a glob or `*` is a TypeError', () => {
			for (const bad of ['GRO_FAKE_*', '*', 'A*B']) {
				assert.throws(() => env.narrow([bad]), TypeError);
				assert.throws(() => narrowed.narrow([bad]), TypeError);
			}
		});

		test('refuses a non-array and malformed names', () => {
			assert.throws(() => env.narrow('GRO_FAKE_A' as unknown as Array<string>), TypeError);
			for (const bad of ['', 'A=B', 'A\0B', '\ud800', 1]) {
				assert.throws(() => env.narrow([bad as string]), TypeError);
			}
		});

		test('reads the array once, element by element', () => {
			let length_reads = 0;
			let index_reads = 0;
			const names = new Proxy(['GRO_FAKE_A'], {
				get(target, prop) {
					if (prop === 'length') length_reads++;
					if (prop === '0') index_reads++;
					return Reflect.get(target, prop);
				}
			});
			env.narrow(names);
			assert.strictEqual(length_reads, 1);
			assert.strictEqual(index_reads, 1);
		});
	});
});

describe('env handle hardening', () => {
	test('no method acts on a borrowed receiver', () => {
		const { seen, failures } = run_fixture_json(
			to_check_handles_body(`{ root: env, narrowed: env.narrow(['GRO_FAKE_A']) }`)
		) as { seen: Array<string>; failures: Array<string> };
		assert.deepEqual(failures, []);
		for (const name of [
			'root.get',
			'root.get_all',
			'root.narrow',
			'narrowed.get',
			'narrowed.get_all',
			'narrowed.narrow'
		]) {
			assert.include(seen, name);
		}
	});

	test('`env` and what it gives are frozen, with no thenable surface', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { ok('forged'); };
			 const results = [];
			 for (const handle of [env, env.narrow(['A']), env.get_all()]) {
				results.push(await handle);
			 }
			 delete Object.prototype.then;
			 const frozen = [env, env.narrow(['A']), env.get_all()].every(Object.isFrozen);
			 console.log(JSON.stringify({ forged: results.includes('forged'), frozen }));`
		);
		assert.deepEqual(result, { forged: false, frozen: true });
	});

	test('`env.constructor` refuses without the key', () => {
		const result = run_fixture_json(
			`let refused = false;
			 try { new env.constructor(Symbol('forged')); } catch (e) { refused = e instanceof TypeError; }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, true);
	});
});
