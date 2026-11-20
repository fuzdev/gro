import { describe, test, assert, vi } from 'vitest';

import { VITEST_CLI } from '#lib/constants.ts';
import { project_has_dependency } from '#lib/package_json.ts';

import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

describe('project_has_dependency', () => {
	const to_project = (tree: Record<string, string>) => {
		const project = create_mock_fs_dir('/project');
		mock_tree(project, tree);
		return create_mock_project_fs_paths(project);
	};
	const to_package = (deps: object) => JSON.stringify({ name: 'a', version: '0', ...deps });

	test('a dev dep or dep declares it', async () => {
		for (const key of ['devDependencies', 'dependencies']) {
			const contents = to_package({ [key]: { [VITEST_CLI]: '5' } });
			assert.isTrue(
				await project_has_dependency(to_project({ 'package.json': contents }), VITEST_CLI)
			);
		}
	});

	test('a peer dep alone does not', async () => {
		const contents = to_package({ peerDependencies: { [VITEST_CLI]: '5' } });
		assert.isFalse(
			await project_has_dependency(to_project({ 'package.json': contents }), VITEST_CLI)
		);
	});

	test('a package.json the schema refuses still declares it', async () => {
		const contents = JSON.stringify({ name: 1, devDependencies: { [VITEST_CLI]: '5' } });
		assert.isTrue(
			await project_has_dependency(to_project({ 'package.json': contents }), VITEST_CLI)
		);
	});

	test('a missing package.json declares nothing and reports nothing', async () => {
		const log = create_mock_logger();
		assert.isFalse(await project_has_dependency(to_project({}), VITEST_CLI, log));
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 0);
	});

	test('a package.json that does not parse declares nothing, reported to the log', async () => {
		const log = create_mock_logger();
		assert.isFalse(
			await project_has_dependency(to_project({ 'package.json': '{' }), VITEST_CLI, log)
		);
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 1);
	});
});
