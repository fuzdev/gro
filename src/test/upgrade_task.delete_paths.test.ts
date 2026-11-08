import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import type { Args } from '#lib/upgrade.task.ts';

import { create_mock_task_context } from './test_helpers.ts';

/*

Hermetic: the cwd moves to a sandbox project and the modules reload,
so `fs.project` is the sandbox, and no deletion reaches the repo.
The install, the sync, and `package.json` are mocked.
`process.chdir` needs Vitest's default `forks` pool.

*/

vi.mock('#lib/npm_install_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/npm_install_helpers.ts')>()),
	install_with_cache_healing_or_throw: vi.fn(async () => undefined)
}));

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	spawn_cli: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({ name: 'a', version: '0', dependencies: {} }))
}));

let base: string;
let project: string;
let outside: string;
let cwd: string;
let upgrade: typeof import('#lib/upgrade.task.ts');

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-upgrade-')));
	project = join(base, 'project');
	outside = join(base, 'outside');
	mkdirSync(join(project, 'node_modules/x'), { recursive: true });
	writeFileSync(join(project, 'package-lock.json'), '{}');
	mkdirSync(join(outside, 'node_modules'), { recursive: true });
	writeFileSync(join(outside, 'package-lock.json'), '{}');
	process.chdir(project);
	vi.resetModules();
	upgrade = await import('#lib/upgrade.task.ts');
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const ARGS_DEFAULT: Args = {
	_: [],
	only: [],
	origin: 'origin',
	force: false,
	pull: false,
	'no-pull': true,
	delete_node_modules: false,
	node_modules_path: 'node_modules',
	delete_lockfile: false,
	lockfile_path: 'package-lock.json',
	dry: true
};

const run = async (args: Partial<Args>): Promise<void> => {
	await upgrade.task.run(create_mock_task_context<Args>(args, {}, ARGS_DEFAULT));
};

const assert_refused = async (args: Partial<Args>): Promise<void> => {
	try {
		await run(args);
		assert.fail('expected a refusal');
	} catch (error) {
		assert.include(String(error), 'resolves outside the project');
	}
};

describe('gro upgrade deleting paths', () => {
	test('deletes node_modules and the lockfile in the project', async () => {
		await run({ delete_node_modules: true, delete_lockfile: true });
		assert.ok(!existsSync(join(project, 'node_modules')));
		assert.ok(!existsSync(join(project, 'package-lock.json')));
	});

	test('refuses paths outside the project, relative or absolute', async () => {
		await assert_refused({
			delete_node_modules: true,
			node_modules_path: '../outside/node_modules'
		});
		await assert_refused({
			delete_node_modules: true,
			node_modules_path: join(outside, 'node_modules')
		});
		await assert_refused({ delete_lockfile: true, lockfile_path: '../outside/package-lock.json' });
		await assert_refused({
			delete_lockfile: true,
			lockfile_path: join(outside, 'package-lock.json')
		});
		assert.ok(existsSync(join(outside, 'node_modules')));
		assert.ok(existsSync(join(outside, 'package-lock.json')));
	});

	test('refuses the project itself and a path through a symlink out of it', async () => {
		symlinkSync(outside, join(project, 'out'));
		await assert_refused({ delete_node_modules: true, node_modules_path: '.' });
		await assert_refused({ delete_node_modules: true, node_modules_path: 'out/node_modules' });
		assert.ok(existsSync(join(project, 'package-lock.json')));
		assert.ok(existsSync(join(outside, 'node_modules')));
	});

	test('a symlinked node_modules is removed itself, its target kept', async () => {
		rmSync(join(project, 'node_modules'), { recursive: true });
		symlinkSync(join(outside, 'node_modules'), join(project, 'node_modules'));
		await run({ delete_node_modules: true });
		assert.throws(() => lstatSync(join(project, 'node_modules')));
		assert.ok(existsSync(join(outside, 'node_modules')));
	});
});
