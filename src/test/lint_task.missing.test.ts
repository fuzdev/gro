import { test, assert, vi } from 'vitest';

import { run_lint } from '#lib/lint.task.ts';
import { is_task_error } from '#lib/task_error.ts';
import type { SpawnProgram } from '#lib/spawn.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';

const to_project = (package_json: object) => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, {
		'package.json': JSON.stringify({ name: 'a', version: '0', ...package_json })
	});
	return create_mock_project_fs_paths(project);
};

const lint_error = async (
	package_json: object,
	found: SpawnProgram | null
): Promise<{ message: string; finds: Array<Array<string>> }> => {
	const find = vi.fn((_role: string) => found);
	try {
		await run_lint(
			{ _: [] },
			{ raw_rest_args: [] },
			{ fs_paths: to_project(package_json), programs: { find }, log: create_mock_logger() }
		);
	} catch (error) {
		assert(is_task_error(error));
		return { message: error.message, finds: find.mock.calls };
	}
	assert.fail('expected a TaskError');
};

// stands in for an `eslint` found on `PATH`, never run when the project doesn't declare it
const FOUND = {} as SpawnProgram;

test('eslint not declared fails without looking for it, so one on `PATH` never runs', async () => {
	const { message, finds } = await lint_error({}, FOUND);
	assert.strictEqual(message, 'no linter found, add eslint to devDependencies');
	assert.deepEqual(finds, []);
});

test('eslint only peered counts as not declared', async () => {
	const { message } = await lint_error({ peerDependencies: { eslint: '9' } }, FOUND);
	assert.strictEqual(message, 'no linter found, add eslint to devDependencies');
});

test('eslint declared but not installed gives the install hint', async () => {
	for (const key of ['devDependencies', 'dependencies']) {
		const { message, finds } = await lint_error({ [key]: { eslint: '9' } }, null);
		assert.strictEqual(message, 'eslint is a dependency but not installed; run `npm i`?');
		assert.deepEqual(finds, [['eslint_cli']]);
	}
});
