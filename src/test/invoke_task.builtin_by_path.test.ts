import { test, assert, afterEach, beforeEach } from 'vitest';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { invoke_task } from '#lib/invoke_task.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { is_reported_error } from '#lib/task_error.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';

import { create_project_fs_paths } from './fs_test_helpers.ts';
import { create_throwing_builtin_host_root, paths } from './test_helpers.ts';

let dir: string;

beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), 'gro_builtin_by_path_'));
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

/**
 * Invokes `task_name` in the project at `root`, returning the message of the task's failure.
 */
const to_refusal = async (task_name: string, root: string): Promise<string> => {
	try {
		await invoke_task(
			task_name,
			{},
			{
				config: create_empty_gro_config(),
				fs_paths: await create_project_fs_paths(root),
				raw_rest_args: [],
				import_module: import_module_node,
				host_root: create_throwing_builtin_host_root()
			}
		);
	} catch (error) {
		assert(is_reported_error(error));
		assert.instanceOf(error.cause, Error);
		return error.cause.message;
	}
	assert.fail('expected the task to be refused');
};

test("a builtin's file run by path is refused, naming the builtin", async () => {
	const message = await to_refusal('src/lib/check.task.ts', paths.root);
	assert.include(message, 'declares `grants`, so it runs only as a builtin, by name');
	assert.include(message, '`gro check`');
});

test("a project task re-exporting a builtin's `task` is refused, naming the builtin", async () => {
	mkdirSync(join(dir, 'src'));
	const check_url = pathToFileURL(join(paths.lib, 'check.task.ts')).href;
	writeFileSync(join(dir, 'src/check.task.ts'), `export { task } from '${check_url}';\n`);
	const message = await to_refusal('src/check.task.ts', dir);
	assert.include(message, 'runs only as a builtin, by name');
	assert.include(message, '`gro check`');
});
