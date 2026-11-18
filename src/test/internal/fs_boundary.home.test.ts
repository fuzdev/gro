import { describe, test, assert, afterEach } from 'vitest';
import { existsSync, mkdirSync, realpathSync, symlinkSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';
import { FsError } from '#lib/fs_error.ts';
import { is_protected_dir, to_protected_dirs } from '#lib/internal/fs_boundary.ts';
import { assert_fs_error_rejects } from '../fs_test_helpers.ts';

/*

The working directory and `$HOME` an `FsPaths` protects are the ones its root passes,
`$HOME`'s real path resolved through the `Fs` given.

*/

let base: string | undefined;

afterEach(() => {
	if (base) rmSync(base, { recursive: true, force: true });
	base = undefined;
});

const create_base = (): string => {
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-home-')));
	return base;
};

describe('to_protected_dirs and `$HOME`', () => {
	test('a symlinked `$HOME` protects its real path too, read through the `Fs` given', async () => {
		const base = create_base();
		mkdirSync(join(base, 'real_home'));
		symlinkSync(join(base, 'real_home'), join(base, 'home'));
		const resolved: Array<string> = [];
		const dirs = await to_protected_dirs(
			{
				...fs,
				real_path: async (path) => {
					resolved.push(path);
					return fs.real_path(path);
				}
			},
			join(base, 'project'),
			{ cwd: join(base, 'cwd'), home: join(base, 'home') }
		);
		assert.deepEqual(dirs, [
			join(base, 'cwd'),
			join(base, 'project'),
			join(base, 'home'),
			join(base, 'real_home')
		]);
		assert.ok(is_protected_dir(join(base, 'real_home'), dirs));
		assert.deepEqual(resolved, [join(base, 'home')]);
	});

	test('`$HOME` and the cwd are normalized, one cwd for a cwd that is the project', async () => {
		const dirs = await to_protected_dirs(fs, '/srv/p', {
			cwd: '/srv//p/./',
			home: '/nowhere/./gro_home//'
		});
		assert.deepEqual(dirs, ['/srv/p', '/nowhere/gro_home']);
	});

	test('an unset, empty, or relative `$HOME` protects nothing and reads nothing', async () => {
		for (const home of [undefined, '', 'home', './home', '~']) {
			const dirs = await to_protected_dirs(
				{
					...fs,
					real_path: async (path) => {
						throw new Error(`read ${path}`);
					}
				},
				'/srv/p',
				{ cwd: '/srv/c', home }
			);
			assert.deepEqual(dirs, ['/srv/c', '/srv/p'], String(home));
		}
	});
});

describe('create_fs_paths and its options', () => {
	test('a `$HOME` whose real path fails to resolve refuses the removal before it acts', async () => {
		const base = create_base();
		mkdirSync(join(base, 'project'));
		mkdirSync(join(base, 'target/keep'), { recursive: true });
		const fs_paths = create_fs_paths(
			{
				...fs,
				project: await fs.open_dir(join(base, 'project')),
				real_path: async (path) => {
					if (path === join(base, 'home')) throw new FsError('not_granted', path);
					return fs.real_path(path);
				}
			},
			{ cwd: join(base, 'project'), home: join(base, 'home') }
		);
		await assert_fs_error_rejects(fs_paths.empty_dir_path(join(base, 'target')), 'not_granted');
		await assert_fs_error_rejects(
			fs_paths.remove_path(join(base, 'target'), { recursive: true }),
			'not_granted'
		);
		assert.ok(realpathSync(join(base, 'target/keep')));
	});

	test("every failure of `$HOME`'s real path refuses the removal, but for a missing `$HOME`", async () => {
		const base = create_base();
		mkdirSync(join(base, 'project'));
		const kinds = [
			'not_found',
			'not_a_directory',
			'permission_denied',
			'not_granted',
			'io_error'
		] as const;
		for (const kind of kinds) {
			mkdirSync(join(base, 'target'));
			const fs_paths = create_fs_paths(
				{
					...fs,
					project: await fs.open_dir(join(base, 'project')),
					real_path: async (path) => {
						if (path === join(base, 'home')) throw new FsError(kind, path);
						return fs.real_path(path);
					}
				},
				{ cwd: join(base, 'project'), home: join(base, 'home') }
			);
			if (kind === 'not_found' || kind === 'not_a_directory') {
				await fs_paths.remove_path(join(base, 'target'), { recursive: true });
				assert.ok(!existsSync(join(base, 'target')), kind);
			} else {
				await assert_fs_error_rejects(
					fs_paths.remove_path(join(base, 'target'), { recursive: true }),
					kind
				);
				assert.ok(existsSync(join(base, 'target')), kind);
				rmSync(join(base, 'target'), { recursive: true });
			}
		}
	});

	test('a missing `$HOME` protects only its path as given', async () => {
		const base = create_base();
		mkdirSync(join(base, 'project'));
		mkdirSync(join(base, 'other'));
		const fs_paths = create_fs_paths(
			{ ...fs, project: await fs.open_dir(join(base, 'project')) },
			{ cwd: join(base, 'project'), home: join(base, 'missing_home') }
		);
		await fs_paths.remove_path(join(base, 'other'), { recursive: true });
		assert.ok(!existsSync(join(base, 'other')));
	});

	test('refuses a cwd that is not an absolute path', () => {
		for (const cwd of ['', '.', 'srv/c', './srv', undefined, null, 1]) {
			assert.throws(
				() => create_fs_paths(fs, { cwd: cwd as string, home: undefined }),
				TypeError,
				'an `FsPaths` takes an absolute `cwd`',
				String(cwd)
			);
		}
	});

	test('refuses removing the cwd and `$HOME` it was given, and their ancestors', async () => {
		const base = create_base();
		mkdirSync(join(base, 'project'));
		mkdirSync(join(base, 'given_cwd/keep'), { recursive: true });
		mkdirSync(join(base, 'real_home/keep'), { recursive: true });
		symlinkSync(join(base, 'real_home'), join(base, 'home'));
		const fs_paths = create_fs_paths(
			{ ...fs, project: await fs.open_dir(join(base, 'project')) },
			{ cwd: join(base, 'given_cwd'), home: join(base, 'home') }
		);
		for (const path of [join(base, 'given_cwd'), join(base, 'real_home'), join(base, 'home/')]) {
			await assert_fs_error_rejects(fs_paths.remove_path(path, { recursive: true }), 'io_error');
		}
		await assert_fs_error_rejects(fs_paths.empty_dir_path(join(base, 'given_cwd')), 'io_error');
		// beside them, a sibling is removed
		mkdirSync(join(base, 'other'));
		await fs_paths.remove_path(join(base, 'other'), { recursive: true });
		assert.ok(realpathSync(join(base, 'given_cwd/keep')));
		assert.ok(realpathSync(join(base, 'real_home/keep')));
	});
});
