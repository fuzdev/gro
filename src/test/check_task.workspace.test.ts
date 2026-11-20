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

describe('gro check --workspace', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('warns and passes with no git on PATH, the check not having run', async () => {
		const deps = create_deps(null);
		await run(deps);
		assert.include(vi.mocked(deps.log.warn).mock.calls[0]?.[0] as string, 'git not found');
		assert.deepEqual(vi.mocked(deps.programs.find).mock.calls, [['git']]);
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
		let error: unknown;
		try {
			await run(create_deps(create_mock_git_program()));
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'git_check_clean_workspace');
	});
});
