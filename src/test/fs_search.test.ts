import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { mkdirSync, symlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import type { FsDir } from '#lib/fs.ts';
import { fs_search } from '#lib/fs_search.ts';
import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

let sandbox: FsSandbox;
let dir: FsDir;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	dir = sandbox.dir;
	const { root } = sandbox;
	writeFileSync(join(root, 'a.ts'), '');
	mkdirSync(join(root, 'b/c'), { recursive: true });
	writeFileSync(join(root, 'b/b.ts'), '');
	writeFileSync(join(root, 'b/c/c.md'), '');
	mkdirSync(join(root, 'empty'));
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

describe('fs_search', () => {
	test('finds files recursively, sorted by path, with absolute ids', async () => {
		const found = await fs_search(dir);
		assert.deepEqual(found, [
			{ path: 'a.ts', id: join(sandbox.root, 'a.ts'), is_directory: false, kind: 'file' },
			{ path: 'b/b.ts', id: join(sandbox.root, 'b/b.ts'), is_directory: false, kind: 'file' },
			{ path: 'b/c/c.md', id: join(sandbox.root, 'b/c/c.md'), is_directory: false, kind: 'file' }
		]);
	});

	test('searches beneath `path`, relative to it', async () => {
		const found = await fs_search(dir, { path: 'b/' });
		assert.deepEqual(
			found.map((f) => [f.path, f.id]),
			[
				['b.ts', join(sandbox.root, 'b/b.ts')],
				['c/c.md', join(sandbox.root, 'b/c/c.md')]
			]
		);
	});

	test('a missing path finds nothing', async () => {
		assert.deepEqual(await fs_search(dir, { path: 'missing' }), []);
		assert.deepEqual(await fs_search(dir, { path: 'missing/deeper' }), []);
	});

	test('a file at the path throws `not_a_directory`', async () => {
		await assert_fs_error_rejects(fs_search(dir, { path: 'a.ts' }), 'not_a_directory');
	});

	test('a path outside the handle throws `outside_root`', async () => {
		await assert_fs_error_rejects(fs_search(dir, { path: '../outside' }), 'outside_root');
	});

	test('`id_dir` names the ids and what the filters see', async () => {
		const seen: Array<string> = [];
		const found = await fs_search(dir, {
			path: 'b',
			id_dir: '/named/b',
			filter: (id) => {
				seen.push(id);
				return true;
			}
		});
		assert.deepEqual(
			found.map((f) => f.id),
			['/named/b/b.ts', '/named/b/c/c.md']
		);
		assert.deepEqual(seen.sort(), ['/named/b/b.ts', '/named/b/c', '/named/b/c/c.md']);
	});

	test('`filter` gets ids and whether each is a directory, and prunes a failing directory', async () => {
		const calls: Array<[string, boolean]> = [];
		const found = await fs_search(dir, {
			filter: (id, is_directory) => {
				calls.push([id.slice(sandbox.root.length), is_directory]);
				return !id.endsWith('/c');
			}
		});
		assert.deepEqual(
			found.map((f) => f.path),
			['a.ts', 'b/b.ts']
		);
		assert.deepEqual(
			calls.sort(([a], [b]) => a.localeCompare(b)),
			[
				['/a.ts', false],
				['/b', true],
				['/b/b.ts', false],
				['/b/c', true],
				['/empty', true]
			]
		);
	});

	test('every `filter` and `file_filter` must pass', async () => {
		const found = await fs_search(dir, {
			filter: [() => true, (id) => !id.endsWith('a.ts')],
			file_filter: [(id) => id.endsWith('.ts'), () => true]
		});
		assert.deepEqual(
			found.map((f) => f.path),
			['b/b.ts']
		);
	});

	test('empty filter arrays filter nothing', async () => {
		const found = await fs_search(dir, { filter: [], file_filter: [] });
		assert.strictEqual(found.length, 3);
	});

	test('a symlink is a result like a file, never followed', async () => {
		const { root } = sandbox;
		symlinkSync('b', join(root, 'link_to_dir'));
		symlinkSync('a.ts', join(root, 'link_to_file'));
		symlinkSync('missing', join(root, 'dangling'));
		symlinkSync(sandbox.outside, join(root, 'escaping'));
		const found = await fs_search(dir);
		const by_path = new Map(found.map((f) => [f.path, f.is_directory]));
		const kinds = new Map(found.map((f) => [f.path, f.kind]));
		assert.strictEqual(kinds.get('a.ts'), 'file');
		for (const link of ['link_to_dir', 'link_to_file', 'dangling', 'escaping']) {
			assert.strictEqual(kinds.get(link), 'symlink');
		}
		assert.strictEqual(by_path.get('link_to_dir'), false);
		assert.strictEqual(by_path.get('link_to_file'), false);
		assert.strictEqual(by_path.get('dangling'), false);
		assert.strictEqual(by_path.get('escaping'), false);
		assert.ok(!found.some((f) => f.path.startsWith('link_to_dir/')));
		assert.ok(!found.some((f) => f.path.startsWith('escaping/')));
	});

	test('unsorted', async () => {
		const unsorted = await fs_search(dir, { sort: false });
		assert.deepEqual(unsorted.map((f) => f.path).sort(), ['a.ts', 'b/b.ts', 'b/c/c.md']);
	});

	test('searches a read-only handle', async () => {
		const found = await fs_search(dir.readonly(), { path: 'b/c' });
		assert.deepEqual(
			found.map((f) => f.path),
			['c.md']
		);
	});
});
