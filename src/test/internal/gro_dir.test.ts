import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
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

/*

Hermetic: the cwd moves to a sandbox project and the modules reload,
so `fs.project` and `paths` are the sandbox, and no write reaches the repo.
`process.chdir` needs Vitest's default `forks` pool.

*/

let base: string;
let project: string;
let cwd: string;
let gro_dir: typeof import('#lib/internal/gro_dir.ts');

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-gro-dir-')));
	project = join(base, 'project');
	mkdirSync(project);
	process.chdir(project);
	vi.resetModules();
	gro_dir = await import('#lib/internal/gro_dir.ts');
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
	assert.strictEqual((await import('#lib/paths.ts')).paths.build, project + '/.gro/');
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const read_marker = (dir: string): string =>
	readFileSync(join(dir, gro_dir.GRO_DIR_MARKER_FILENAME), 'utf8');

describe('gro_dir_ensure', () => {
	test('makes a missing `.gro/` with its marker', async () => {
		await gro_dir.gro_dir_ensure();
		assert.strictEqual(read_marker(join(project, '.gro')), gro_dir.GRO_DIR_MARKER_CONTENT);
	});

	test('never marks a `.gro/` that exists, whatever it holds', async () => {
		mkdirSync(join(project, '.gro'));
		await gro_dir.gro_dir_ensure();
		assert.deepEqual(readdirSync(join(project, '.gro')), []);
		writeFileSync(join(project, '.gro/build.json'), '{}');
		await gro_dir.gro_dir_ensure();
		assert.deepEqual(readdirSync(join(project, '.gro')), ['build.json']);
	});

	test('makes and marks the missing target of a linked `.gro/`, its parents too', async () => {
		const target = join(base, 'tmpfs/a/gro');
		symlinkSync(target, join(project, '.gro'));
		await gro_dir.gro_dir_ensure();
		assert.deepEqual(readdirSync(target), [gro_dir.GRO_DIR_MARKER_FILENAME]);
	});

	test('never marks the existing target of a linked `.gro/`, even an empty one', async () => {
		for (const name of ['empty', 'sibling']) {
			const target = join(base, name);
			mkdirSync(target);
			if (name === 'sibling') writeFileSync(join(target, 'index.html'), 'sibling');
			rmSync(join(project, '.gro'), { force: true });
			symlinkSync(target, join(project, '.gro'));
			await gro_dir.gro_dir_ensure();
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
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(join(base, 'repo')));
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(join(base, 'repo/a/b')));
		assert.ok(await gro_dir.gro_dir_is_in_git_work_tree(join(base, 'worktree')));
		assert.ok(!(await gro_dir.gro_dir_is_in_git_work_tree(join(base, 'plain'))));
	});
});
