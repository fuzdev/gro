import { describe, test, assert, vi, beforeEach } from 'vitest';

import { run_check, type Args, type CheckDeps } from '#lib/check.task.ts';
import { TaskError } from '#lib/task_error.ts';
import type { SpawnProgram } from '#lib/spawn.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

const ARGS = {
	typecheck: false,
	test: false,
	gen: false,
	format: false,
	package_json: false,
	lint: false,
	build: false,
	sync: false,
	install: false,
	workspace: true
} as unknown as Args;

const create_deps = (git: SpawnProgram | null): CheckDeps => ({
	fs_paths: create_mock_fs_paths(),
	env: { get: () => undefined },
	programs: { find: vi.fn(() => git) },
	invoke_task: vi.fn(),
	log: create_mock_logger()
});

const run = (deps: CheckDeps): Promise<void> =>
	run_check(ARGS, { map_package_json: null, raw_rest_args: [] }, deps);

const get_error = async (deps: CheckDeps): Promise<unknown> => {
	try {
		await run(deps);
	} catch (error) {
		return error;
	}
	return assert.fail('expected the check to fail');
};

describe('gro check --workspace', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test("fails with no git on PATH, as the check it asked for can't run", async () => {
		const deps = create_deps(null);
		const error = await get_error(deps);
		assert.instanceOf(error, TaskError);
		assert.strictEqual(
			error.message,
			'--workspace checks the git workspace and needs git, which was not found on PATH' +
				' - install git or drop --workspace'
		);
		assert.deepEqual(vi.mocked(deps.programs.find).mock.calls, [['git_cli']]);
	});

	test("fails in a directory that isn't in a git repo, rather than reading it as clean", async () => {
		const not_a_repo: SpawnProgram = {
			...create_mock_git_program(),
			out: () =>
				Promise.resolve({
					kind: 'exited',
					code: 128,
					stdout: '',
					stderr: 'fatal: not a git repository (or any of the parent directories): .git\n',
					truncated: false
				})
		};
		const deps = create_deps(not_a_repo);
		const error = await get_error(deps);
		assert.instanceOf(error, TaskError);
		assert.strictEqual(
			error.message,
			"--workspace checks the git workspace, and this directory isn't in a git repo" +
				' - run it in the repo or drop --workspace'
		);
	});

	test('passes a clean workspace without a warning', async () => {
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		const deps = create_deps(create_mock_git_program());
		await run(deps);
		assert.strictEqual(vi.mocked(deps.log.warn).mock.calls.length, 0);
	});

	test('fails a dirty workspace', async () => {
		git_ops.git_check_clean_workspace.mockResolvedValue('git has unstaged changes');
		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 0 });
		const error = await get_error(create_deps(create_mock_git_program()));
		assert.instanceOf(error, TaskError);
		assert.strictEqual(
			error.message,
			'--workspace checks the git workspace, and git has unstaged changes' +
				" - commit them, or run `gro sync` if they're generated"
		);
	});

	test('a dirty workspace with untracked files names removing them too', async () => {
		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 0 });
		// porcelain output, as the mock reads a message holding a NUL
		for (const [porcelain, issues] of [
			['?? new.ts\0', 'untracked files'],
			['MM a.ts\0?? new.ts\0', 'unstaged changes, staged but uncommitted changes, untracked files']
		]) {
			git_ops.git_check_clean_workspace.mockResolvedValue(porcelain!);
			const error = await get_error(create_deps(create_mock_git_program()));
			assert.instanceOf(error, TaskError);
			assert.strictEqual(
				error.message,
				`--workspace checks the git workspace, and git has ${issues}` +
					" - commit them or remove the untracked files, or run `gro sync` if they're generated"
			);
		}
	});
});
