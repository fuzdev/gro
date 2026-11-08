import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { is_fs_error } from '#lib/fs.ts';

/*

Hermetic: the cwd moves to a sandbox project and the modules reload,
so `fs.project` and `paths` are the sandbox, and cleaning never reaches the repo.
`process.chdir` needs Vitest's default `forks` pool.

*/

let base: string;
let project: string;
let outside: string;
let cwd: string;
let clean_fs: (typeof import('#lib/clean_fs.ts'))['clean_fs'];

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-clean-fs-')));
	project = join(base, 'project');
	outside = join(base, 'outside');
	mkdirSync(project);
	mkdirSync(join(outside, 'sub'), { recursive: true });
	writeFileSync(join(outside, 'build.json'), '{}');
	writeFileSync(join(outside, 'sub/a.txt'), 'a');
	process.chdir(project);
	vi.resetModules();
	({ clean_fs } = await import('#lib/clean_fs.ts'));
	// the reloaded modules see the sandbox, so no op reaches the repo
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
	assert.strictEqual((await import('#lib/paths.ts')).paths.root, project + '/');
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const is_link = (path: string): boolean => {
	try {
		return lstatSync(path).isSymbolicLink();
	} catch {
		return false;
	}
};

describe('clean_fs', () => {
	test('removes the cleaned directories, skipping missing ones', async () => {
		mkdirSync(join(project, '.svelte-kit'));
		mkdirSync(join(project, 'build'));
		writeFileSync(join(project, 'build/index.html'), '');
		mkdirSync(join(project, '.gro'));
		writeFileSync(join(project, '.gro/build.json'), '{}');

		await clean_fs({ gro_dir: true, sveltekit: true });

		assert.deepEqual(readdirSync(project), []);
	});

	test('a `.gro/` linked outside the project keeps its link and has its target emptied', async () => {
		symlinkSync(outside, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(is_link(join(project, '.gro')));
		assert.ok(lstatSync(outside).isDirectory());
		assert.deepEqual(readdirSync(outside), []);
	});

	test('a `.gro/` linked inside the project, to a file, dangling, or in a loop is unlinked', async () => {
		mkdirSync(join(project, 'inside'));
		writeFileSync(join(project, 'inside/a.txt'), 'a');
		for (const target of ['inside', join(outside, 'build.json'), 'missing', '.gro']) {
			symlinkSync(target, join(project, '.gro'));

			await clean_fs({ gro_dir: true });

			assert.ok(!is_link(join(project, '.gro')), target);
		}
		assert.ok(existsSync(join(project, 'inside/a.txt')));
		assert.ok(existsSync(join(outside, 'build.json')));
	});

	test('any other symlinked path is unlinked, never followed', async () => {
		symlinkSync(join(outside, 'sub'), join(project, 'build'));
		symlinkSync(join(outside, 'sub'), join(project, 'dist_x'));
		symlinkSync('build', join(project, 'dist'));
		symlinkSync('loop', join(project, 'loop'));
		symlinkSync('loop', join(project, '.svelte-kit'));

		await clean_fs({ sveltekit: true, dist_dirs: true });

		for (const name of ['build', 'dist', 'dist_x', '.svelte-kit']) {
			assert.ok(!is_link(join(project, name)), name);
		}
		assert.ok(existsSync(join(outside, 'sub/a.txt')));
	});

	test('a `.gro/` linked to an ancestor of the project is refused, emptying nothing', async () => {
		symlinkSync(base, join(project, '.gro'));

		try {
			await clean_fs({ gro_dir: true });
			assert.fail('expected a refusal');
		} catch (error) {
			assert.ok(is_fs_error(error, 'io_error'), String(error));
		}

		assert.ok(is_link(join(project, '.gro')));
		assert.ok(existsSync(join(outside, 'sub/a.txt')));
	});
});
