import { describe, test, assert, vi } from 'vitest';

import { cwd } from '#lib/cwd.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const MODULE_URL = to_lib_url('cwd.ts');

const runner = use_fixture_runner(`import {cwd} from ${JSON.stringify(MODULE_URL)};`);
const { run_fixture_json } = runner;

describe('cwd', () => {
	test('`cwd` is the checked working directory', () => {
		assert.deepEqual(run_fixture_json('console.log(JSON.stringify(cwd()));'), runner.sandbox.root);
	});

	test('`cwd` is a frozen function, no constructor, taking no receiver', () => {
		assert.ok(Object.isFrozen(cwd));
		assert.throws(() => new (cwd as unknown as new () => unknown)(), TypeError);
		const unbound = cwd;
		assert.strictEqual(unbound(), cwd());
		assert.strictEqual(cwd.call({}), cwd());
	});
});
