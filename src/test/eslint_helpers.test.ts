import { test, assert } from 'vitest';
import { symlinkSync } from 'node:fs';
import { join } from 'node:path';

import { ESLINT_CONFIG_FILENAMES, find_eslint_config } from '#lib/eslint_helpers.ts';

import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import {
	create_fs_sandbox,
	create_project_fs_paths,
	remove_fs_sandbox
} from './fs_test_helpers.ts';

const to_project = (tree: Record<string, string>) => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, tree);
	return create_mock_project_fs_paths(project);
};

// the flat names from ESLint 9's `config-loader.js`, then `@eslint/eslintrc`'s legacy ones, each in lookup order
test('the names are the flat config names, then the legacy ones', () => {
	assert.deepEqual(ESLINT_CONFIG_FILENAMES, [
		'eslint.config.js',
		'eslint.config.mjs',
		'eslint.config.cjs',
		'eslint.config.ts',
		'eslint.config.mts',
		'eslint.config.cts',
		'.eslintrc.js',
		'.eslintrc.cjs',
		'.eslintrc.yaml',
		'.eslintrc.yml',
		'.eslintrc.json',
		'.eslintrc'
	]);
});

test('finds each name ESLint reads at the root', async () => {
	for (const name of ESLINT_CONFIG_FILENAMES) {
		assert.strictEqual(await find_eslint_config(to_project({ [name]: '' })), name);
	}
});

test('finds the flat config before a legacy one', async () => {
	const fs_paths = to_project({ '.eslintrc.json': '{}', 'eslint.config.ts': '' });
	assert.strictEqual(await find_eslint_config(fs_paths), 'eslint.config.ts');
});

test('a project with no config, or one only below the root, has none', async () => {
	assert.isNull(await find_eslint_config(to_project({ 'package.json': '{}' })));
	assert.isNull(await find_eslint_config(to_project({ 'src/eslint.config.js': '' })));
});

test('a symlinked config counts unfollowed, absolute or dangling', async () => {
	const sandbox = await create_fs_sandbox();
	try {
		symlinkSync(join(sandbox.outside, 'secret.txt'), join(sandbox.root, 'eslint.config.js'));
		symlinkSync('nowhere.js', join(sandbox.root, '.eslintrc.json'));
		const fs_paths = await create_project_fs_paths(sandbox.root);
		assert.strictEqual(await find_eslint_config(fs_paths), 'eslint.config.js');
	} finally {
		remove_fs_sandbox(sandbox);
	}
});

test('a dangling relative symlink counts', async () => {
	const sandbox = await create_fs_sandbox();
	try {
		symlinkSync('nowhere.js', join(sandbox.root, 'eslint.config.js'));
		assert.strictEqual(
			await find_eslint_config(await create_project_fs_paths(sandbox.root)),
			'eslint.config.js'
		);
	} finally {
		remove_fs_sandbox(sandbox);
	}
});
