import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, symlinkSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { fs } from '#lib/fs.ts';
import { create_fs_paths, type FsPaths } from '#lib/fs_paths.ts';

import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

/*

`create_fs_paths` on a sandbox project, with the cwd left where it is,
so a relative path that reached the cwd would show as a path outside the project.
The boundary's resolution itself is tested in `internal/fs_boundary.test.ts`.

*/

let sandbox: FsSandbox;
let fs_paths: FsPaths;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	fs_paths = create_fs_paths({ ...fs, project: sandbox.dir });
	assert.notStrictEqual(process.cwd(), sandbox.root);
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

describe('create_fs_paths', () => {
	test("`project` is the root's project, and the result is frozen", () => {
		assert.strictEqual(fs_paths.project, sandbox.dir);
		assert.ok(Object.isFrozen(fs_paths));
	});

	test('a relative path resolves against the project, not the cwd', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		const opened = await fs_paths.open_path('a.txt');
		assert.strictEqual(opened.dir, fs_paths.project);
		assert.strictEqual(opened.path, 'a.txt');
		assert.strictEqual(await opened.dir.read_text(opened.path), 'a');
		const dir = await fs_paths.open_dir_path('.');
		assert.strictEqual(dir?.dir, fs_paths.project);
		assert.strictEqual(dir?.path, '');
	});

	test('a relative path climbing out of the project opens outside it', async () => {
		const opened = await fs_paths.open_path('../outside/secret.txt');
		assert.notStrictEqual(opened.dir, fs_paths.project);
		assert.strictEqual(await opened.dir.read_text(opened.path), 'secret');
	});

	test('search ids start with the path made absolute against the project', async () => {
		mkdirSync(join(sandbox.root, 'src'));
		writeFileSync(join(sandbox.root, 'src/b.ts'), '');
		const found = await fs_paths.search_path('src');
		assert.deepEqual(
			found.map((f) => f.id),
			[join(sandbox.root, 'src/b.ts')]
		);
	});

	test('removes and empties relative to the project, refusing the project itself', async () => {
		mkdirSync(join(sandbox.root, 'build/sub'), { recursive: true });
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		await fs_paths.empty_dir_path('build');
		assert.ok(existsSync(join(sandbox.root, 'build')));
		assert.ok(!existsSync(join(sandbox.root, 'build/sub')));
		await fs_paths.remove_path('link');
		assert.ok(!existsSync(join(sandbox.root, 'link')));
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
		await assert_fs_error_rejects(fs_paths.remove_path('.', { recursive: true }), 'io_error');
		await assert_fs_error_rejects(fs_paths.empty_dir_path(''), 'io_error');
		assert.ok(existsSync(join(sandbox.root, 'build')));
	});

	test('over another project, still refuses the working directory and its ancestors', () => {
		// a subprocess whose cwd is a sandbox project, so a missed refusal removes only the sandbox
		const cwd_project = join(sandbox.base, 'cwd_project');
		mkdirSync(join(cwd_project, 'keep'), { recursive: true });
		const script = join(sandbox.base, 'script.ts');
		writeFileSync(
			script,
			`import { fs } from ${JSON.stringify(resolve('src/lib/fs.ts'))};
import { create_fs_paths } from ${JSON.stringify(resolve('src/lib/fs_paths.ts'))};
const fs_paths = create_fs_paths({ ...fs, project: await fs.open_dir(process.argv[2]) });
for (const [name, run] of [
	['remove cwd', () => fs_paths.remove_path(process.cwd(), { recursive: true })],
	['empty cwd', () => fs_paths.empty_dir_path(process.cwd())],
	['remove parent', () => fs_paths.remove_path(process.cwd() + '/..', { recursive: true })],
	['remove project', () => fs_paths.remove_path('.', { recursive: true })]
]) {
	try {
		await run();
		console.log(name, 'done');
	} catch (error) {
		console.log(name, error.kind);
	}
}
`
		);
		const result = spawnSync(process.execPath, [script, sandbox.root], {
			cwd: cwd_project,
			encoding: 'utf8'
		});
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(result.stdout.trim().split('\n'), [
			'remove cwd io_error',
			'empty cwd io_error',
			'remove parent io_error',
			'remove project io_error'
		]);
		assert.ok(existsSync(join(cwd_project, 'keep')));
		assert.ok(existsSync(sandbox.root));
	});
});
