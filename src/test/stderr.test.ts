import { describe, test, assert, vi } from 'vitest';

import { to_check_handles_body, to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const { run_fixture, run_fixture_json } = use_fixture_runner(
	`import {stderr} from ${JSON.stringify(to_lib_url('stderr.ts'))};`
);

describe('stderr', () => {
	test('writes to stderr, apart from stdout', () => {
		const result = run_fixture(
			`const ok = await stderr.write('err');
			 console.log(JSON.stringify(ok));`
		);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.strictEqual(result.stderr, 'err');
		assert.strictEqual(result.stdout, 'true\n');
	});

	test('`is_terminal` reports a pipe as no terminal', () => {
		assert.strictEqual(run_fixture_json('console.log(JSON.stringify(stderr.is_terminal));'), false);
	});

	test('no method or getter acts on a borrowed receiver', () => {
		const { seen, failures } = run_fixture_json(to_check_handles_body('{ stderr }')) as {
			seen: Array<string>;
			failures: Array<string>;
		};
		assert.deepEqual(failures, []);
		for (const name of ['stderr.is_terminal', 'stderr.write']) assert.include(seen, name);
	});

	test('is frozen, with no thenable surface', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { ok('forged'); };
			 const awaited = await stderr;
			 delete Object.prototype.then;
			 console.log(JSON.stringify({ forged: awaited === 'forged', frozen: Object.isFrozen(stderr) }));`
		);
		assert.deepEqual(result, { forged: false, frozen: true });
	});
});
