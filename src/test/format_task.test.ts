import { test, assert, vi } from 'vitest';

import { run_format, task, type Args } from '#lib/format.task.ts';
import { format_directory } from '#lib/format_directory.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import { TaskError } from '#lib/task_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

vi.mock('#lib/format_directory.ts', () => ({
	format_directory: vi.fn(async () => ({ ok: true }))
}));

test("`run_format` formats the project's `src/` through the boundary it's given", async () => {
	const fs_paths = create_mock_fs_paths();
	const log = create_mock_logger();
	const search_filters = [() => true];
	vi.mocked(format_directory).mockClear();
	await run_format({ _: ['a.ts'], check: true }, { search_filters }, { fs_paths, log });
	const [given_fs_paths, options] = vi.mocked(format_directory).mock.calls[0]!;
	assert.strictEqual(given_fs_paths, fs_paths);
	assert.deepEqual(options, {
		log,
		dir: '/project/src/',
		check: true,
		filter: search_filters,
		patterns: ['a.ts']
	});
});

test('`run_format` failing is a TaskError naming the check', async () => {
	vi.mocked(format_directory).mockResolvedValueOnce({ ok: false } as never);
	const error = await run_format(
		{ check: true },
		{ search_filters: [] },
		{ fs_paths: create_mock_fs_paths(), log: create_mock_logger() }
	).catch((e: unknown) => e);
	assert.instanceOf(error, TaskError);
	assert.strictEqual(error.message, 'Failed formatting check.');
});

test("`format`'s `run` takes its boundary from its `host`", async () => {
	const fs_paths = create_mock_fs_paths();
	vi.mocked(format_directory).mockClear();
	await task.run({
		task_args: { check: false },
		config: { search_filters: [] },
		log: create_mock_logger(),
		host: { fs_paths }
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>);
	assert.strictEqual(vi.mocked(format_directory).mock.calls[0]![0], fs_paths);
});

test("`format`'s `run` filters by its config's `search_filters`", async () => {
	const search_filters = [() => true];
	vi.mocked(format_directory).mockClear();
	await task.run({
		task_args: { check: true },
		config: { search_filters },
		log: create_mock_logger(),
		host: { fs_paths: create_mock_fs_paths() }
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>);
	assert.strictEqual(vi.mocked(format_directory).mock.calls[0]![1].filter, search_filters);
});
