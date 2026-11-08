import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import type { FsPaths } from '#lib/fs_paths.ts';
import * as gro_dir from '#lib/internal/gro_dir.ts';

import { create_project_fs_paths } from '../fs_test_helpers.ts';

/*

Hermetic: the `FsPaths` is on a sandbox project, which the default `.gro/` is relative to,
so no write reaches the repo.

*/

let base: string;
let project: string;
let fs_paths: FsPaths;

beforeEach(async () => {
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-gro-dir-')));
	project = join(base, 'project');
	mkdirSync(project);
	fs_paths = await create_project_fs_paths(project);
	assert.strictEqual(fs_paths.project.path, project);
});

afterEach(() => {
	rmSync(base, { recursive: true, force: true });
});

const read_marker = (dir: string): string =>
	readFileSync(join(dir, gro_dir.GRO_DIR_MARKER_FILENAME), 'utf8');

describe('gro_dir_ensure', () => {
	test('makes a missing `.gro/` with its marker', async () => {
		await gro_dir.gro_dir_ensure(fs_paths);
		assert.strictEqual(read_marker(join(project, '.gro')), gro_dir.GRO_DIR_MARKER_CONTENT);
	});

	test('never marks a `.gro/` that exists, whatever it holds', async () => {
		mkdirSync(join(project, '.gro'));
		await gro_dir.gro_dir_ensure(fs_paths);
		assert.deepEqual(readdirSync(join(project, '.gro')), []);
		writeFileSync(join(project, '.gro/build.json'), '{}');
		await gro_dir.gro_dir_ensure(fs_paths);
		assert.deepEqual(readdirSync(join(project, '.gro')), ['build.json']);
	});

	test('makes and marks the missing target of a linked `.gro/`, its parents too', async () => {
		const target = join(base, 'tmpfs/a/gro');
		symlinkSync(target, join(project, '.gro'));
		await gro_dir.gro_dir_ensure(fs_paths);
		assert.deepEqual(readdirSync(target), [gro_dir.GRO_DIR_MARKER_FILENAME]);
	});

	test('never marks the existing target of a linked `.gro/`, even an empty one', async () => {
		for (const name of ['empty', 'sibling']) {
			const target = join(base, name);
			mkdirSync(target);
			if (name === 'sibling') writeFileSync(join(target, 'index.html'), 'sibling');
			rmSync(join(project, '.gro'), { force: true });
			symlinkSync(target, join(project, '.gro'));
			await gro_dir.gro_dir_ensure(fs_paths);
			assert.notInclude(readdirSync(target), gro_dir.GRO_DIR_MARKER_FILENAME, name);
		}
	});
});

describe('gro_dir_is_in_git_work_tree', () => {
	test('finds a `.git` directory or file at the path or above it', async () => {
		mkdirSync(join(base, 'repo/.git'), { recursive: true });
		mkdirSync(join(base, 'repo/a/b'), { recursive: true });
		mkdirSync(join(base, 'worktree'));
		writeFileSync(join(base, 'worktree/.git'), 'gitdir: elsewhere\n');
		mkdirSync(join(base, 'plain'));
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(fs_paths, join(base, 'repo')));
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(fs_paths, join(base, 'repo/a/b')));
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(fs_paths, join(base, 'worktree')));
		assert.ok(!(await gro_dir.gro_dir_is_in_git_work_tree(fs_paths, join(base, 'plain'))));
	});
});
