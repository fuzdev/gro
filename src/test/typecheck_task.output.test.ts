import { describe, test, assert, vi, afterEach } from 'vitest';
import { create_deferred, wait } from '@fuzdev/fuz_util/async.ts';

import type { SpawnResult } from '#lib/spawn.ts';
import { is_task_error } from '#lib/task_error.ts';
import { task } from '#lib/typecheck.task.ts';

import { stdout as stdout_module, type ProcessOutput } from '#lib/stdout.ts';
import { stderr as stderr_module } from '#lib/stderr.ts';

import { create_mock_child, create_mock_stream, get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_builtin_context } from './test_helpers.ts';

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
	const run = Promise.resolve(
		task.run(
			await create_mock_builtin_context(task, ARGS, {
				stdout: stdout_module,
				stderr: stderr_module
			})
		)
	).finally(() => {
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

test("svelte-check's output goes to the host's streams, the working directory replaced", async () => {
	const decoder = new TextDecoder();
	const to_capture = (written: Array<string>): ProcessOutput => ({
		is_terminal: false,
		write: async (data: string | Uint8Array) => {
			written.push(typeof data === 'string' ? data : decoder.decode(data));
			return true;
		}
	});
	const out: Array<string> = [];
	const err: Array<string> = [];
	const stdout = create_mock_stream();
	const stderr = create_mock_stream();
	const mock = await get_mock_spawn();
	mock.available = (name) => name === 'node_modules/.bin/svelte-check';
	mock.create_child = () =>
		create_mock_child({
			closed: Promise.resolve(OK),
			stdout: stdout.stream,
			stderr: stderr.stream
		});
	const run = Promise.resolve(
		task.run(
			await create_mock_builtin_context(task, ARGS, {
				stdout: to_capture(out),
				stderr: to_capture(err)
			})
		)
	);
	await wait(0);
	stdout.push(new TextEncoder().encode('error in /project/src/a.ts\n'));
	stderr.push(new TextEncoder().encode('warning in /project/src/b.ts\n'));
	stdout.end();
	stderr.end();
	await run;
	assert.include(out.join(''), 'error in ./src/a.ts');
	assert.notInclude(out.join(''), 'warning');
	assert.include(err.join(''), 'warning in ./src/b.ts');
});
