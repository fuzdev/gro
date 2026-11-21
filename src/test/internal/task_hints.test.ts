import { test, assert } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { find_task_file_for_input, to_task_path_hint } from '#lib/internal/task_hints.ts';
import { create_project_fs_paths } from '../fs_test_helpers.ts';

// the tests run in Gro's own repo, the project of these `FsPaths`
const fs_paths = await create_project_fs_paths(process.cwd());

test('a hint names the task file an input likely meant and the directory it lists', async () => {
	assert.strictEqual(
		await find_task_file_for_input('src/lib/check', fs_paths),
		'src/lib/check.task.ts'
	);
	assert.strictEqual(
		await to_task_path_hint('src/lib/check', fs_paths),
		'Run the task file with `gro task src/lib/check.task.ts`.'
	);
	assert.strictEqual(
		await to_task_path_hint('src/lib', fs_paths),
		"List the directory's tasks with `gro task src/lib`."
	);
	assert.strictEqual(await to_task_path_hint('src/lib', fs_paths, false), null);
	assert.strictEqual(
		await to_task_path_hint('src/no_such_dir/x', fs_paths),
		'Nothing is at `src/no_such_dir/x`, `src/no_such_dir/x.task.ts`, or `src/no_such_dir/x.task.js`.'
	);
	// a path through a file names nothing
	assert.strictEqual(
		await to_task_path_hint('package.json/x', fs_paths),
		'Nothing is at `package.json/x`, `package.json/x.task.ts`, or `package.json/x.task.js`.'
	);
});

test('a hint stats a path outside the project as one inside it', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_task_hints_'));
	try {
		writeFileSync(join(dir, 'greet.task.ts'), '');
		assert.strictEqual(
			await to_task_path_hint(join(dir, 'greet'), fs_paths),
			`Run the task file with \`gro task ${join(dir, 'greet')}.task.ts\`.`
		);
		assert.strictEqual(
			await to_task_path_hint(dir, fs_paths),
			`List the directory's tasks with \`gro task ${dir}\`.`
		);
	} finally {
		rmSync(dir, { recursive: true });
	}
});
