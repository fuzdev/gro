import { test, expect, vi } from 'vitest';
import { resolve } from 'node:path';

import { task, type Args } from '#lib/gen.task.ts';
import { find_genfiles } from '#lib/gen.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn(async () => ({ ok: false, type: 'input_directories_with_no_files' }))
}));

test('explicit relative input paths resolve against the project root', async () => {
	await task.run(
		create_mock_task_context<Args>({
			_: ['./src/foo', 'src/bar'],
			root_dirs: [process.cwd()],
			check: false
		})
	);
	expect(vi.mocked(find_genfiles).mock.calls[0]![1]).toEqual([resolve('src/foo'), 'src/bar']);
});
