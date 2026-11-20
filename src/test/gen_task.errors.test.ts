import { test, assert, vi } from 'vitest';

import { run_gen_task, type GenTaskDeps } from '#lib/gen.task.ts';
import type { GenResults } from '#lib/gen.ts';
import { run_gen } from '#lib/run_gen.ts';
import { TaskError } from '#lib/task_error.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';

import { create_mock_logger, create_mock_timings } from './test_helpers.ts';
import { to_fs_paths_options } from './fs_test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn(async () => ({ ok: true, value: { resolved_input_files: [] } })),
	load_genfiles: vi.fn(async () => ({ ok: true, value: { modules: [] } }))
}));

vi.mock('#lib/run_gen.ts', () => ({ run_gen: vi.fn() }));

const ID = '/p/src/a.gen.ts';

/**
 * Runs the task to a single genfile failing with `error`,
 * returning everything it logged about the failure.
 */
const run_failing = async (error: Error): Promise<string> => {
	const failure = { ok: false as const, id: ID, reason: 'failed', error, elapsed: 1 };
	vi.mocked(run_gen).mockResolvedValue({
		results: [failure],
		successes: [],
		failures: [failure],
		input_count: 1,
		output_count: 0,
		elapsed: 1
	} satisfies GenResults);
	const deps: GenTaskDeps = {
		fs_paths: create_fs_paths(fs, to_fs_paths_options()),
		import_module: vi.fn(),
		invoke_project_task: vi.fn(),
		load_svelte_config: vi.fn(),
		log: create_mock_logger(),
		timings: create_mock_timings()
	};
	let thrown: unknown;
	try {
		await run_gen_task(
			{ _: [], root_dirs: [process.cwd()], check: false },
			{ config: create_empty_gro_config() },
			deps
		);
	} catch (e) {
		thrown = e;
	}
	assert.ok(thrown instanceof TaskError);
	assert.include(thrown.message, 'Failed to generate 1 file');
	return [...vi.mocked(deps.log.info).mock.calls, ...vi.mocked(deps.log.error).mock.calls]
		.flat()
		.join('\n');
};

test('a genfile failing with a `TaskError` prints its message and no stack', async () => {
	const output = await run_failing(new TaskError('the message with the fix'));
	assert.equal(output.split('the message with the fix').length - 1, 2);
	assert.notMatch(output, /^\s+at /m);
});

test('a genfile failing with any other error keeps its stack', async () => {
	const output = await run_failing(new Error('unexpected'));
	assert.match(output, /^\s+at /m);
});

test('a genfile failing with an error that has a cause prints the cause after the stack, in both reports', async () => {
	const output = await run_failing(
		new Error('io_error: /p/x', { cause: new Error('too many symlinks at /p/x') })
	);
	assert.match(output, /^\s+at /m);
	// the table's line and the failure's
	assert.equal(output.split('\ncause: too many symlinks at /p/x').length - 1, 2, output);
});
