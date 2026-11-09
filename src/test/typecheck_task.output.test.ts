import { describe, test, assert, vi, afterEach } from 'vitest';
import { create_deferred, wait } from '@fuzdev/fuz_util/async.ts';

import type { SpawnResult } from '#lib/spawn.ts';
import { is_task_error } from '#lib/task.ts';
import { task } from '#lib/typecheck.task.ts';

import { create_mock_child, create_mock_stream, get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(async () => undefined)
}));

afterEach(async () => {
	vi.restoreAllMocks();
	(await get_mock_spawn()).reset();
});

const ARGS = {
	svelte_check_cli: 'svelte-check',
	typescript_cli: 'tsc',
	path_replacement: '.',
	cwd: '/project'
};

const OK: SpawnResult = { kind: 'exited', code: 0 };

/**
 * Starts the task with svelte-check as a child the test finishes by hand.
 */
const start = async () => {
	vi.spyOn(process.stdout, 'write').mockImplementation(() => true);
	vi.spyOn(process.stderr, 'write').mockImplementation(() => true);
	const closed = create_deferred<SpawnResult>();
	const stdout = create_mock_stream();
	const stderr = create_mock_stream();
	const mock = await get_mock_spawn();
	mock.available = (name) => name === 'node_modules/.bin/svelte-check';
	mock.create_child = () =>
		create_mock_child({ closed: closed.promise, stdout: stdout.stream, stderr: stderr.stream });
	let settled = false;
	const run = Promise.resolve(task.run(create_mock_task_context(ARGS))).finally(() => {
		settled = true;
	});
	await wait(0);
	return {
		closed,
		end_output: () => {
			stdout.end();
			stderr.end();
		},
		run,
		is_settled: () => settled
	};
};

describe('typecheck task waits for svelte-check', () => {
	test('on the output after the child closes', async () => {
		const { closed, end_output, run, is_settled } = await start();
		closed.resolve(OK);
		await wait(0);
		assert.ok(!is_settled(), 'output is still open');
		end_output();
		await run;
	});

	test('on the child closing after the output ends', async () => {
		const { closed, end_output, run, is_settled } = await start();
		end_output();
		await wait(0);
		assert.ok(!is_settled(), 'the child is still open');
		closed.resolve(OK);
		await run;
	});

	test('a failure is reported after the output is written', async () => {
		const { closed, end_output, run, is_settled } = await start();
		closed.resolve({ kind: 'exited', code: 2 });
		await wait(0);
		assert.ok(!is_settled(), 'output is still open');
		end_output();
		const error = await run.then(
			() => assert.fail('expected a failure'),
			(e: unknown) => e
		);
		assert.ok(is_task_error(error));
		assert.strictEqual(error.message, 'Failed to typecheck. code 2');
	});
});
