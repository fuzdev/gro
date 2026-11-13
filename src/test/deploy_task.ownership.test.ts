import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import { execFileSync } from 'node:child_process';
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import type { Args } from '#lib/deploy.task.ts';

import { create_mock_deploy_task_context } from './deploy_task_test_helpers.ts';

/*

Hermetic and unmocked: the cwd moves to a sandbox project, a real git repo, and the modules
reload, so `fs.project` and the deploy task's cwd are the sandbox. Every case is refused by the
deploy dir's ownership check, which runs before any git op but its read-only queries,
so nothing here reaches past the sandbox. `process.chdir` needs Vitest's default `forks` pool.

*/

let base: string;
let project: string;
let sibling: string;
let cwd: string;
let deploy_task: (typeof import('#lib/deploy.task.ts'))['task'];

// the fixture's repos are made without the host's git config
const git = (dir: string, ...args: Array<string>): void => {
	execFileSync('git', ['-C', dir, ...args], {
		stdio: 'ignore',
		env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' }
	});
};

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-deploy-owner-')));
	project = join(base, 'project');
	sibling = join(base, 'sibling');
	for (const [dir, origin] of [
		[project, 'git@example.com:me/project.git'],
		[sibling, 'git@example.com:me/sibling.git']
	] as const) {
		mkdirSync(dir);
		git(dir, 'init', '-q');
		git(dir, 'remote', 'add', 'origin', origin);
	}
	writeFileSync(join(sibling, 'index.html'), 'sibling');
	mkdirSync(join(sibling, 'sub'));
	writeFileSync(join(sibling, 'sub/a.txt'), 'a');
	process.chdir(project);
	vi.resetModules();
	({ task: deploy_task } = await import('#lib/deploy.task.ts'));
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const assert_refused = async (
	deploy_dir: string,
	message: string,
	args: Partial<Args> = {}
): Promise<void> => {
	const ctx = create_mock_deploy_task_context({ deploy_dir, dry: true, build: false, ...args });
	try {
		await deploy_task.run(ctx);
		assert.fail('expected a refusal');
	} catch (error) {
		assert.include(String(error), message);
	}
};

describe('deploy dir ownership, with real git', () => {
	test('a sibling repo is refused and left untouched', async () => {
		await assert_refused(
			sibling,
			"its `origin` remote is git@example.com:me/sibling.git, not the project's"
		);
		await assert_refused(join(sibling, 'sub'), `inside the git repo at ${sibling}`);
		assert.strictEqual(readFileSync(join(sibling, 'index.html'), 'utf8'), 'sibling');
		assert.strictEqual(readFileSync(join(sibling, 'sub/a.txt'), 'utf8'), 'a');
		assert.ok(existsSync(join(sibling, '.git')));
	});

	test('a plain directory beneath `.gro/`, inside the project repo, is refused and kept', async () => {
		mkdirSync(join(project, '.gro/deploy'), { recursive: true });
		writeFileSync(join(project, '.gro/deploy/keep.txt'), 'keep');
		await assert_refused('.gro/deploy', `inside the git repo at ${project}`);
		assert.strictEqual(readFileSync(join(project, '.gro/deploy/keep.txt'), 'utf8'), 'keep');
	});

	test('a directory outside any repo is refused and kept', async () => {
		const plain = join(base, 'plain');
		mkdirSync(plain);
		writeFileSync(join(plain, 'keep.txt'), 'keep');
		await assert_refused(plain, 'it is not in a git repo');
		assert.ok(existsSync(join(plain, 'keep.txt')));
	});

	// a repo of its own with the project's origin, like a second checkout of the project
	const create_same_origin_repo = (path: string, tagged: boolean): void => {
		mkdirSync(path, { recursive: true });
		git(path, 'init', '-q');
		git(path, 'remote', 'add', 'origin', 'git@example.com:me/project.git');
		writeFileSync(join(path, 'keep.txt'), 'keep');
		if (tagged) git(path, 'config', '--local', 'gro.deploy-clone', 'true');
	};

	test('a same-origin checkout without the deploy-clone tag is refused and kept', async () => {
		const checkout = join(base, 'checkout');
		create_same_origin_repo(checkout, false);
		await assert_refused(checkout, 'lacks the local git config `gro.deploy-clone true`');
		assert.strictEqual(readFileSync(join(checkout, 'keep.txt'), 'utf8'), 'keep');
	});

	test("a tagged clone with the project's origin passes the ownership check", async () => {
		const clone = join(project, '.gro/deploy');
		create_same_origin_repo(clone, true);
		// a custom target without `--force` is the next refusal, after the ownership check
		// and before any git op, so reaching it shows the clone was accepted
		await assert_refused('.gro/deploy', 'custom target branch', { target: 'other' });
		assert.strictEqual(readFileSync(join(clone, 'keep.txt'), 'utf8'), 'keep');
	});

	test('a bare repo whose work tree is itself, with a forged tag, is refused and kept', async () => {
		// the layout a committed bare repo could have: toplevel, origin, and tag all pass
		const forged = join(project, '.gro/deploy');
		mkdirSync(join(project, '.gro'));
		git(project, 'init', '-q', '--bare', forged);
		git(forged, 'config', 'core.bare', 'false');
		git(forged, 'config', 'core.worktree', forged);
		git(forged, 'remote', 'add', 'origin', 'git@example.com:me/project.git');
		git(forged, 'config', '--local', 'gro.deploy-clone', 'true');
		writeFileSync(join(forged, 'keep.txt'), 'keep');
		await assert_refused('.gro/deploy', `its git dir is ${forged}, not its own ${forged}/.git`);
		assert.strictEqual(readFileSync(join(forged, 'keep.txt'), 'utf8'), 'keep');
	});

	test('a deploy dir that is a file is refused and kept', async () => {
		mkdirSync(join(project, '.gro'));
		writeFileSync(join(project, '.gro/deploy'), 'file');
		await assert_refused('.gro/deploy', 'it is not a directory');
		assert.strictEqual(readFileSync(join(project, '.gro/deploy'), 'utf8'), 'file');
	});
});
