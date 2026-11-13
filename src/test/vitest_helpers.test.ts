import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import { chmodSync, mkdirSync, symlinkSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { VITEST_CLI } from '#lib/constants.ts';
import { find_test_files, project_has_vitest } from '#lib/vitest_helpers.ts';

import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import {
	create_fs_sandbox,
	assert_fs_error_rejects,
	create_project_fs_paths,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';
import { create_mock_logger } from './test_helpers.ts';

describe('project_has_vitest', () => {
	const to_project = (tree: Record<string, string>) => {
		const project = create_mock_fs_dir('/project');
		mock_tree(project, tree);
		return create_mock_project_fs_paths(project);
	};
	const to_package = (deps: object) => JSON.stringify({ name: 'a', version: '0', ...deps });

	test('a dev dep or dep declares it', async () => {
		for (const key of ['devDependencies', 'dependencies']) {
			const contents = to_package({ [key]: { [VITEST_CLI]: '5' } });
			assert.isTrue(await project_has_vitest(to_project({ 'package.json': contents })));
		}
	});

	test('a peer dep alone does not', async () => {
		const contents = to_package({ peerDependencies: { [VITEST_CLI]: '5' } });
		assert.isFalse(await project_has_vitest(to_project({ 'package.json': contents })));
	});

	test('a package.json the schema refuses still declares it', async () => {
		const contents = JSON.stringify({ name: 1, devDependencies: { [VITEST_CLI]: '5' } });
		assert.isTrue(await project_has_vitest(to_project({ 'package.json': contents })));
	});

	test('a missing package.json declares nothing and reports nothing', async () => {
		const log = create_mock_logger();
		assert.isFalse(await project_has_vitest(to_project({}), log));
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 0);
	});

	test('a package.json that does not parse declares nothing, reported to the log', async () => {
		const log = create_mock_logger();
		assert.isFalse(await project_has_vitest(to_project({ 'package.json': '{' }), log));
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 1);
	});
});

describe('find_test_files', () => {
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

	const find_paths = async (
		files: Array<string>,
		dir: string = join(sandbox.root, 'src/')
	): Promise<Array<string>> => {
		write_files(sandbox.root, files);
		const found = await find_test_files(await create_project_fs_paths(sandbox.root), dir);
		return found.map((f) => f.path);
	};

	test('finds what the pattern and the default include match, sorted, at any depth', async () => {
		assert.deepEqual(
			await find_paths([
				'src/z.test.ts',
				'src/a/b/c.test.js',
				'src/m.test.mts',
				'src/n.test.cjs',
				'src/t.test.tsx',
				'src/a.svelte.test.ts'
			]),
			['a.svelte.test.ts', 'a/b/c.test.js', 'm.test.mts', 'n.test.cjs', 't.test.tsx', 'z.test.ts']
		);
	});

	test('skips what the default include or the pattern would drop', async () => {
		assert.deepEqual(
			await find_paths([
				'src/plain.ts',
				'src/plain.spec.ts',
				'src/data.test.json',
				'src/foo.test.svelte.ts',
				'src/up.TEST.ts',
				'src/x.test./ts'
			]),
			[]
		);
	});

	test('finds a `.spec.` file whose path the pattern matches, the pattern ignoring case', async () => {
		assert.deepEqual(await find_paths(['src/a.test.d/s.spec.ts', 'src/B.TEST.D/s.spec.ts']), [
			'a.test.d/s.spec.ts',
			'B.TEST.D/s.spec.ts'
		]);
	});

	test('skips `node_modules` and `.git` at any depth', async () => {
		assert.deepEqual(
			await find_paths([
				'src/node_modules/x.test.ts',
				'src/lib/node_modules/y.test.ts',
				'src/.git/z.test.ts',
				'src/a.test.ts'
			]),
			['a.test.ts']
		);
	});

	test('finds tests beneath directories that Gro search filters skip but Vitest runs', async () => {
		assert.deepEqual(
			await find_paths([
				'src/build/b.test.ts',
				'src/dist/d.test.ts',
				'src/.hidden/h.test.ts',
				'src/.dotfile.test.ts'
			]),
			['.dotfile.test.ts', '.hidden/h.test.ts', 'build/b.test.ts', 'dist/d.test.ts']
		);
	});

	test('skips only a whole `node_modules` or `.git` segment', async () => {
		assert.deepEqual(await find_paths(['src/my_node_modules/m.test.ts', 'src/a.git/g.test.ts']), [
			'a.git/g.test.ts',
			'my_node_modules/m.test.ts'
		]);
	});

	test('finds a symlinked test file, which Vitest runs', async () => {
		write_files(sandbox.root, ['outside/real.ts', 'src/b.ts']);
		symlinkSync('../outside/real.ts', join(sandbox.root, 'src/linked.test.ts'));
		const found = await find_test_files(
			await create_project_fs_paths(sandbox.root),
			join(sandbox.root, 'src/')
		);
		assert.deepEqual(
			found.map((f) => [f.path, f.kind]),
			[['linked.test.ts', 'symlink']]
		);
	});

	test.skipIf(process.getuid?.() === 0)(
		'a listing that fails throws rather than finding nothing',
		async () => {
			write_files(sandbox.root, ['src/locked/a.test.ts']);
			const locked = join(sandbox.root, 'src/locked');
			chmodSync(locked, 0o000);
			try {
				const fs_paths = await create_project_fs_paths(sandbox.root);
				let error: unknown = null;
				try {
					await find_test_files(fs_paths, join(sandbox.root, 'src/'));
				} catch (e) {
					error = e;
				}
				assert.isNotNull(error);
			} finally {
				chmodSync(locked, 0o755);
			}
		}
	);

	test('follows a symlinked directory, out of the project too', async () => {
		write_files(sandbox.base, ['outside/o.test.ts', 'outside/plain.spec.ts']);
		write_files(sandbox.root, ['src/a.ts']);
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/linked'));
		const found = await find_test_files(
			await create_project_fs_paths(sandbox.root),
			join(sandbox.root, 'src/')
		);
		// the id names the file through the link, as Vitest and the error message do
		assert.deepEqual(
			found.map((f) => [f.path, f.id, f.kind]),
			[['linked/o.test.ts', join(sandbox.root, 'src/linked/o.test.ts'), 'file']]
		);
	});

	test('a symlinked directory named with the pattern matches what is beneath it', async () => {
		write_files(sandbox.base, ['outside/plain.spec.ts']);
		write_files(sandbox.root, ['src/a.ts']);
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/linked.test.d'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/plain_link'));
		assert.deepEqual(await find_paths([]), ['linked.test.d/plain.spec.ts']);
	});

	test('a symlinked directory beneath a name with the pattern matches what is beneath it', async () => {
		write_files(sandbox.base, ['outside/plain.spec.ts']);
		write_files(sandbox.root, ['src/a.test.d/x.ts']);
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/a.test.d/link'));
		assert.deepEqual(await find_paths([]), ['a.test.d/link/plain.spec.ts']);
	});

	test('a symlink loop ends, each real directory searched once', async () => {
		write_files(sandbox.root, ['src/a.test.ts', 'src/sub/s.test.ts']);
		symlinkSync('..', join(sandbox.root, 'src/sub/loop'));
		symlinkSync('../..', join(sandbox.root, 'src/sub/loop2'));
		assert.deepEqual(await find_paths([]), ['a.test.ts', 'sub/s.test.ts']);
	});

	test('a loop through a name with the pattern is searched once more, as Vitest does', async () => {
		write_files(sandbox.root, ['src/q.spec.ts', 'src/sub/r.spec.ts']);
		symlinkSync('..', join(sandbox.root, 'src/sub/x.test.d'));
		assert.deepEqual(await find_paths([]), [
			'sub/x.test.d/q.spec.ts',
			'sub/x.test.d/sub/r.spec.ts'
		]);
	});

	test('skips a dangling symlink and a link named `node_modules`', async () => {
		write_files(sandbox.base, ['outside/o.test.ts']);
		write_files(sandbox.root, ['src/a.ts']);
		symlinkSync('missing.ts', join(sandbox.root, 'src/dangling.test.ts'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'src/node_modules'));
		assert.deepEqual(await find_paths([]), []);
	});

	test('a `dir` that is a file throws', async () => {
		write_files(sandbox.root, ['src']);
		const fs_paths = await create_project_fs_paths(sandbox.root);
		await assert_fs_error_rejects(
			find_test_files(fs_paths, join(sandbox.root, 'src/')),
			'not_a_directory'
		);
	});

	test('does not search outside `dir`', async () => {
		assert.deepEqual(await find_paths(['test/z.test.ts', 'z.test.ts', 'src/lib/a.ts']), []);
	});

	test('a project beneath a `node_modules` directory still finds its tests', async () => {
		assert.deepEqual(
			await find_paths(['node_modules/p/src/a.test.ts'], join(sandbox.root, 'node_modules/p/src/')),
			['a.test.ts']
		);
	});

	test('a missing `dir` finds nothing', async () => {
		assert.deepEqual(await find_paths([], join(sandbox.root, 'missing/')), []);
	});
});
