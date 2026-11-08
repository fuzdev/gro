import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { is_fs_error } from '#lib/fs.ts';
import { GRO_DIR_MARKER_CONTENT, GRO_DIR_MARKER_FILENAME } from '#lib/internal/gro_dir.ts';

import { create_mock_logger } from './test_helpers.ts';

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
let gro_dir_ensure: (typeof import('#lib/internal/gro_dir.ts'))['gro_dir_ensure'];

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
	({ gro_dir_ensure } = await import('#lib/internal/gro_dir.ts'));
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

	test('a `.gro/` linked to a marked directory outside the project keeps its link and has its target emptied', async () => {
		writeFileSync(join(outside, GRO_DIR_MARKER_FILENAME), GRO_DIR_MARKER_CONTENT);
		symlinkSync(outside, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(is_link(join(project, '.gro')));
		assert.ok(lstatSync(outside).isDirectory());
		// emptied, the marker rewritten, so the next clean empties it too
		assert.deepEqual(readdirSync(outside), [GRO_DIR_MARKER_FILENAME]);
		assert.strictEqual(
			readFileSync(join(outside, GRO_DIR_MARKER_FILENAME), 'utf8'),
			GRO_DIR_MARKER_CONTENT
		);
	});

	test('a `.gro/` linked to an unmarked directory outside the project is unlinked, its target untouched', async () => {
		symlinkSync(outside, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(!is_link(join(project, '.gro')));
		assert.deepEqual(readdirSync(outside).sort(), ['build.json', 'sub']);
		assert.ok(existsSync(join(outside, 'sub/a.txt')));
	});

	test('a `.gro/` linked to a sibling repo is unlinked, the repo untouched', async () => {
		const sibling = join(base, 'sibling');
		mkdirSync(join(sibling, '.git'), { recursive: true });
		writeFileSync(join(sibling, 'index.html'), 'sibling');
		symlinkSync(sibling, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(!is_link(join(project, '.gro')));
		assert.deepEqual(readdirSync(sibling).sort(), ['.git', 'index.html']);
		assert.strictEqual(readFileSync(join(sibling, 'index.html'), 'utf8'), 'sibling');
	});

	test('a `.gro/` linked to an empty unmarked directory is unlinked with a warning, never marked', async () => {
		const empty = join(base, 'empty');
		mkdirSync(empty);
		symlinkSync(empty, join(project, '.gro'));
		const log = create_mock_logger();

		await clean_fs({ gro_dir: true, log });

		assert.ok(!is_link(join(project, '.gro')));
		assert.deepEqual(readdirSync(empty), []);
		assert.strictEqual(vi.mocked(log.warn).mock.calls.length, 1);
		const message = String(vi.mocked(log.warn).mock.calls[0]![0]);
		assert.include(message, `has no \`${GRO_DIR_MARKER_FILENAME}\``);
		assert.include(message, 'a path that does not exist yet');
	});

	test('a dangling `.gro/` link is left alone, and Gro recreates and marks its target', async () => {
		// a tmpfs target gone after a reboot
		const gone = join(base, 'tmpfs/gone');
		mkdirSync(join(base, 'tmpfs'));
		for (const target of [gone, 'missing_inside']) {
			rmSync(join(project, '.gro'), { force: true });
			symlinkSync(target, join(project, '.gro'));
			const log = create_mock_logger();

			await clean_fs({ gro_dir: true, log });

			assert.ok(is_link(join(project, '.gro')), target);
			assert.strictEqual(vi.mocked(log.warn).mock.calls.length, 0, target);
		}
		rmSync(join(project, '.gro'));
		symlinkSync(gone, join(project, '.gro'));
		await gro_dir_ensure();
		assert.deepEqual(readdirSync(gone), [GRO_DIR_MARKER_FILENAME]);
	});

	test('a `.gro/` linked to a missing path is made and marked by Gro, then emptied by clean', async () => {
		const fresh = join(base, 'tmpfs/fresh');
		mkdirSync(join(base, 'tmpfs'));
		symlinkSync(fresh, join(project, '.gro'));
		await gro_dir_ensure();
		writeFileSync(join(fresh, 'build.json'), '{}');

		await clean_fs({ gro_dir: true });

		assert.ok(is_link(join(project, '.gro')));
		assert.deepEqual(readdirSync(fresh), [GRO_DIR_MARKER_FILENAME]);
	});

	test('a marked target inside a git work tree is unlinked with a warning, never emptied', async () => {
		const repo = join(base, 'repo');
		mkdirSync(join(repo, '.git'), { recursive: true });
		mkdirSync(join(repo, 'cache'));
		writeFileSync(join(repo, 'cache', GRO_DIR_MARKER_FILENAME), GRO_DIR_MARKER_CONTENT);
		writeFileSync(join(repo, 'cache/tracked.txt'), 'tracked');
		symlinkSync(join(repo, 'cache'), join(project, '.gro'));
		const log = create_mock_logger();

		await clean_fs({ gro_dir: true, log });

		assert.ok(!is_link(join(project, '.gro')));
		assert.strictEqual(readFileSync(join(repo, 'cache/tracked.txt'), 'utf8'), 'tracked');
		const message = String(vi.mocked(log.warn).mock.calls[0]![0]);
		assert.include(message, 'inside a git work tree');
		assert.include(message, 'outside any git work tree');
	});

	test('a marker that is a symlink marks nothing', async () => {
		writeFileSync(join(base, 'marker'), GRO_DIR_MARKER_CONTENT);
		symlinkSync(join(base, 'marker'), join(outside, GRO_DIR_MARKER_FILENAME));
		symlinkSync(outside, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(!is_link(join(project, '.gro')));
		assert.ok(existsSync(join(outside, 'sub/a.txt')));
	});

	test('a `.gro/` linked inside the project, to a file, or in a loop is unlinked', async () => {
		mkdirSync(join(project, 'inside'));
		writeFileSync(join(project, 'inside/a.txt'), 'a');
		for (const target of ['inside', join(outside, 'build.json'), '.gro']) {
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

	test('a `.gro/` linked to an unmarked ancestor of the project is unlinked, emptying nothing', async () => {
		symlinkSync(base, join(project, '.gro'));

		await clean_fs({ gro_dir: true });

		assert.ok(!is_link(join(project, '.gro')));
		assert.ok(existsSync(join(project)));
		assert.ok(existsSync(join(outside, 'sub/a.txt')));
	});

	test('a `.gro/` linked to a marked ancestor of the project is refused, emptying nothing', async () => {
		writeFileSync(join(base, GRO_DIR_MARKER_FILENAME), GRO_DIR_MARKER_CONTENT);
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
