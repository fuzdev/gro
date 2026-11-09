import { test, assert, describe, vi } from 'vitest';

import { sveltekit_sync_if_obviously_needed } from '#lib/sveltekit_helpers.ts';
import {
	SVELTEKIT_DEP_NAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_TSCONFIG_PATH
} from '#lib/constants.ts';

import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

const DECLARED = JSON.stringify({ devDependencies: { [SVELTEKIT_DEP_NAME]: '3' } });

const to_project = (tree: Record<string, string>) => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, tree);
	return create_mock_project_fs_paths(project);
};

const count_syncs = (spawn: ReturnType<typeof create_mock_spawn>): number =>
	spawn.calls.filter((call) => call.method === 'run' && call.args[0] === 'sync').length;

describe('sveltekit_sync_if_obviously_needed', () => {
	test('a synced project reads no package.json', async () => {
		const fs_paths = to_project({
			'package.json': DECLARED,
			[`${SVELTEKIT_DEV_DIRNAME}/x`]: '',
			[SVELTEKIT_TSCONFIG_PATH]: '{}'
		});
		const open_path = vi.spyOn(fs_paths, 'open_path');
		const spawn = create_mock_spawn();
		await sveltekit_sync_if_obviously_needed(fs_paths, spawn);
		assert.strictEqual(count_syncs(spawn), 0);
		for (const [path] of open_path.mock.calls) {
			assert.notInclude(path, 'package.json');
		}
	});

	// either path missing means a sync, `npm install` wiping only the generated tsconfig
	test.each([
		['only `.svelte-kit`', `${SVELTEKIT_DEV_DIRNAME}/x`],
		['only the generated tsconfig', SVELTEKIT_TSCONFIG_PATH]
	])('syncs a declared project with %s', async (_name, present) => {
		const spawn = create_mock_spawn();
		await sveltekit_sync_if_obviously_needed(
			to_project({ 'package.json': DECLARED, [present]: '{}' }),
			spawn
		);
		assert.strictEqual(count_syncs(spawn), 1);
	});
});
