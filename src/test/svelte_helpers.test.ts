import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { mkdirSync, symlinkSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { find_svelte_files } from '#lib/svelte_helpers.ts';

import {
	create_fs_sandbox,
	assert_fs_error_rejects,
	create_project_fs_paths,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

describe('find_svelte_files', () => {
	let sandbox: FsSandbox;

	beforeEach(async () => {
		sandbox = await create_fs_sandbox();
	});

	afterEach(() => {
		remove_fs_sandbox(sandbox);
	});

	const write_files = (root: string, files: Array<string>): void => {
		for (const file of files) {
			const path = join(root, file);
			mkdirSync(dirname(path), { recursive: true });
			writeFileSync(path, '');
		}
	};

	const find = async (dir: string = join(sandbox.root, 'src/')) =>
		find_svelte_files(await create_project_fs_paths(sandbox.root), dir);

	const find_paths = async (files: Array<string>): Promise<Array<string>> => {
		write_files(sandbox.root, files);
		return (await find()).map((f) => f.path);
	};

	test('finds `.svelte` files at any depth, never one TypeScript wildcards skip', async () => {
		assert.deepEqual(
			await find_paths([
				'src/A.svelte',
				'src/lib/b/B.svelte',
				'src/lib/state.svelte.ts',
				'src/node_modules/p/N.svelte',
				'src/lib/.git/G.svelte',
				'src/.hidden/H.svelte',
				'src/lib/.Dot.svelte',
				'src/bower_components/p/B.svelte',
				'src/jspm_packages/p/J.svelte',
				'C.svelte'
			]),
			['A.svelte', 'lib/b/B.svelte']
		);
	});

	test('follows a symlinked directory, out of the project too, as svelte-check does', async () => {
		write_files(sandbox.base, ['outside/O.svelte', 'outside/o.ts']);
		write_files(sandbox.root, ['src/a.ts']);
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/linked'));
		const found = await find();
		// the id names the file through the link, as the error message does
		assert.deepEqual(
			found.map((f) => [f.path, f.id, f.kind]),
			[['linked/O.svelte', join(sandbox.root, 'src/linked/O.svelte'), 'file']]
		);
	});

	test('a link to a file counts by its own name, wherever it points', async () => {
		write_files(sandbox.root, ['outside_src/real.txt', 'outside_src/Real.svelte', 'src/a.ts']);
		symlinkSync('../outside_src/real.txt', join(sandbox.root, 'src/Linked.svelte'));
		symlinkSync('../outside_src/Real.svelte', join(sandbox.root, 'src/linked.ts'));
		const found = await find();
		assert.deepEqual(
			found.map((f) => [f.path, f.kind]),
			[['Linked.svelte', 'symlink']]
		);
	});

	test('skips a link that does not resolve, dangling or a loop of its own', async () => {
		mkdirSync(join(sandbox.root, 'src'));
		symlinkSync('Missing.svelte', join(sandbox.root, 'src/Dangling.svelte'));
		symlinkSync('Loop.svelte', join(sandbox.root, 'src/Loop.svelte'));
		assert.deepEqual(await find_paths([]), []);
	});

	test('a loop of directory links ends, each real directory searched once', async () => {
		write_files(sandbox.root, ['src/A.svelte', 'src/sub/S.svelte']);
		symlinkSync('..', join(sandbox.root, 'src/sub/loop'));
		symlinkSync('../..', join(sandbox.root, 'src/sub/loop2'));
		assert.deepEqual(await find_paths([]), ['A.svelte', 'sub/S.svelte']);
	});

	test('a directory reached by two paths is named by the first in name order', async () => {
		write_files(sandbox.root, ['src/z/Z.svelte']);
		symlinkSync('z', join(sandbox.root, 'src/a_link'));
		assert.deepEqual(await find_paths([]), ['a_link/Z.svelte']);
	});

	test('a link with an excluded name is not followed or counted', async () => {
		write_files(sandbox.base, ['outside/O.svelte']);
		mkdirSync(join(sandbox.root, 'src'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/node_modules'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/.git'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/.linked'));
		symlinkSync(join(sandbox.outside, 'O.svelte'), join(sandbox.root, 'src/.L.svelte'));
		assert.deepEqual(await find_paths([]), []);
	});

	test('a `dir` that is a file throws, and a missing one finds nothing', async () => {
		write_files(sandbox.root, ['src']);
		await assert_fs_error_rejects(find(), 'not_a_directory');
		assert.deepEqual(await find(join(sandbox.root, 'missing/')), []);
	});
});
