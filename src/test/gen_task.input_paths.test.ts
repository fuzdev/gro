import { describe, test, assert, expect, vi, beforeEach } from 'vitest';
import { resolve } from 'node:path';

import { run_gen_task, type GenTaskDeps } from '#lib/gen.task.ts';
import { find_genfiles } from '#lib/gen.ts';
import { SOURCE_DIRNAME } from '#lib/constants.ts';
import { TaskError } from '#lib/task_error.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';

import { create_mock_logger, create_mock_timings } from './test_helpers.ts';
import { to_fs_paths_options } from './fs_test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn()
}));

const REASON = 'Input directory contains no matching files: src/foo';

const to_deps = (): GenTaskDeps => ({
	fs_paths: create_fs_paths(fs, to_fs_paths_options()),
	import_module: vi.fn(),
	// Gro's own repo, synced, so no `svelte-kit` is found
	programs: { find: vi.fn(() => null) },
	load_svelte_config: vi.fn(),
	log: create_mock_logger(),
	timings: create_mock_timings()
});

const run = (input_paths: Array<string>, check = false, deps = to_deps()): Promise<void> =>
	run_gen_task(
		{ _: input_paths, root_dirs: [process.cwd()], check },
		{ config: create_empty_gro_config() },
		deps
	);

const to_error = (input_paths: Array<string>, check?: boolean): Promise<unknown> =>
	run(input_paths, check).then(
		() => undefined,
		(e: unknown) => e
	);

beforeEach(() => {
	vi.mocked(find_genfiles).mockReset();
	vi.mocked(find_genfiles).mockResolvedValue({
		ok: false,
		type: 'input_directories_with_no_files',
		input_directories_with_no_files: [],
		resolved_input_files: [],
		resolved_input_paths: [],
		reasons: [REASON]
	});
});

test('explicit relative input paths resolve against the project root', async () => {
	await to_error(['./src/foo', 'src/bar']);
	expect(vi.mocked(find_genfiles).mock.calls[0]![1]).toEqual([resolve('src/foo'), 'src/bar']);
});

describe('finding no genfiles', () => {
	for (const check of [false, true]) {
		test(`the default input passes, doing nothing (check: ${check})`, async () => {
			assert.isUndefined(await to_error([SOURCE_DIRNAME], check));
		});

		test(`a named directory with none fails, naming it (check: ${check})`, async () => {
			const deps = to_deps();
			const error = await run(['src/foo'], check, deps).catch((e: unknown) => e);
			assert(error instanceof TaskError, 'expected a TaskError');
			assert.include(error.message, 'Failed to find gen modules');
			expect(deps.log.error).toHaveBeenCalledWith(expect.stringContaining(REASON));
		});
	}

	test('the default input beside another path is a named input', async () => {
		assert.instanceOf(await to_error([SOURCE_DIRNAME, 'src/foo']), TaskError);
	});
});
