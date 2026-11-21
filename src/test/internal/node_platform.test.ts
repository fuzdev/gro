import { test, assert } from 'vitest';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	create_node_platform,
	load_gro_package,
	to_gro_package_dir
} from '#lib/internal/node_platform.ts';
import { create_project_fs_paths } from '../fs_test_helpers.ts';
import { paths } from '../test_helpers.ts';

test('to_gro_package_dir finds the package dir from source or dist under any directory name', () => {
	assert.strictEqual(
		to_gro_package_dir('/home/me/dev/gro/src/lib/internal/node_platform.ts'),
		'/home/me/dev/gro/'
	);
	assert.strictEqual(
		to_gro_package_dir('/a/.claude/worktrees/wt1/src/lib/internal/node_platform.ts'),
		'/a/.claude/worktrees/wt1/'
	);
	assert.strictEqual(
		to_gro_package_dir('/a/node_modules/@fuzdev/gro/dist/internal/node_platform.js'),
		'/a/node_modules/@fuzdev/gro/'
	);
	assert.strictEqual(
		to_gro_package_dir('/a/my_fork/dist/internal/node_platform.js'),
		'/a/my_fork/'
	);
});

test("the Node platform names this repo as the running Gro, with its package.json's version", async () => {
	const platform = await create_node_platform(await create_project_fs_paths(paths.root));
	assert.strictEqual(platform.gro.dir, paths.root);
	assert.strictEqual(
		platform.gro.version,
		JSON.parse(readFileSync(join(paths.root, 'package.json'), 'utf8')).version
	);
});

test("load_gro_package reads the version, '' when the package.json has none", async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_package_')) + '/';
	try {
		const fs_paths = await create_project_fs_paths(dir);
		writeFileSync(join(dir, 'package.json'), '{"version": "1.2.3"}');
		assert.deepEqual(await load_gro_package(fs_paths, dir), { version: '1.2.3', dir });
		writeFileSync(join(dir, 'package.json'), '{"name": "x"}');
		assert.deepEqual(await load_gro_package(fs_paths, dir), { version: '', dir });
	} finally {
		rmSync(dir, { recursive: true });
	}
});
