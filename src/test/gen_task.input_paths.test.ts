import { describe, test, assert, expect, vi, beforeEach } from 'vitest';
import { resolve } from 'node:path';

import { task, type Args } from '#lib/gen.task.ts';
import { find_genfiles } from '#lib/gen.ts';
import { SOURCE_DIRNAME } from '#lib/constants.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn()
}));

const REASON = 'Input directory contains no matching files: src/foo';

const to_ctx = (input_paths: Array<string>, check = false) =>
	create_mock_task_context<Args>({ _: input_paths, root_dirs: [process.cwd()], check });

const to_error = (input_paths: Array<string>, check?: boolean): Promise<unknown> =>
	Promise.resolve(task.run(to_ctx(input_paths, check))).then(
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
			const ctx = to_ctx(['src/foo'], check);
			const error = await Promise.resolve(task.run(ctx)).catch((e: unknown) => e);
			assert(error instanceof TaskError, 'expected a TaskError');
			assert.include(error.message, 'Failed to find gen modules');
			expect(ctx.log.error).toHaveBeenCalledWith(expect.stringContaining(REASON));
		});
	}

	test('the default input beside another path is a named input', async () => {
		assert.instanceOf(await to_error([SOURCE_DIRNAME, 'src/foo']), TaskError);
	});
});
