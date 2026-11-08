import { describe, test, assert } from 'vitest';

import {
	git_check_setting_pull_rebase,
	git_check_workspace,
	git_checkout,
	git_current_branch_first_commit_hash,
	git_current_branch_name,
	git_current_commit_hash,
	git_delete_local_branch,
	git_fetch,
	git_local_branch_exists,
	git_pull,
	git_push,
	git_push_to_create,
	git_remote_branch_exists,
	GIT_LOCAL_PASS_ENV,
	to_git_program
} from '#lib/git.ts';
import type { SpawnOutput, SpawnProgram, SpawnResult } from '#lib/spawn.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_spawn } from './spawn_mocks.ts';

interface Call {
	method: 'run' | 'out';
	args: ReadonlyArray<string>;
}

const EXITED_OK: SpawnResult = { kind: 'exited', code: 0 };

const to_output = (result: SpawnResult, stdout = ''): SpawnOutput => ({
	...result,
	stdout,
	stderr: '',
	truncated: false
});

/**
 * A program that records how it was called, resolving `run` to `run_result`
 * and `out` to `out_output`.
 */
const create_recording_program = (
	run_result: SpawnResult = EXITED_OK,
	out_output: SpawnOutput = to_output(EXITED_OK, 'main\n')
): { program: SpawnProgram; calls: Array<Call> } => {
	const calls: Array<Call> = [];
	return {
		calls,
		program: {
			name: 'git',
			path: '/mock/git',
			run: async (args) => {
				calls.push({ method: 'run', args });
				return run_result;
			},
			out: async (args) => {
				calls.push({ method: 'out', args });
				return out_output;
			},
			start: () => {
				throw Error('no start');
			}
		}
	};
};

describe('git argv and how each command runs', () => {
	test('a command that reaches a remote or deletes a branch runs with `run`', async () => {
		const cases: Array<[(git: SpawnProgram) => Promise<unknown>, ReadonlyArray<string>]> = [
			[(git) => git_fetch(git, 'origin', 'main'), ['fetch', '--end-of-options', 'origin', 'main']],
			[(git) => git_pull(git, 'origin', 'main'), ['pull', 'origin', 'main']],
			[(git) => git_push(git, 'origin', 'main'), ['push', '--end-of-options', 'origin', 'main']],
			[
				(git) => git_push(git, 'origin', 'main', undefined, true),
				['push', '-u', '--end-of-options', 'origin', 'main']
			],
			[
				(git) => git_remote_branch_exists(git, 'origin', 'main'),
				['ls-remote', '--exit-code', '--heads', '--end-of-options', 'origin', 'refs/heads/main']
			],
			[(git) => git_delete_local_branch(git, 'old'), ['branch', '-D', '--end-of-options', 'old']]
		];
		for (const [call, args] of cases) {
			const { program, calls } = create_recording_program();
			await call(program);
			assert.deepEqual(calls, [{ method: 'run', args }]);
		}
	});

	test('git_checkout reads the branch with `out` and checks out with `run`', async () => {
		const { program, calls } = create_recording_program();
		assert.strictEqual(await git_checkout(program, 'other'), 'main');
		assert.deepEqual(calls, [
			{ method: 'out', args: ['rev-parse', '--abbrev-ref', 'HEAD'] },
			{ method: 'run', args: ['checkout', '--end-of-options', 'other'] }
		]);
	});

	test('git_push_to_create runs the existence check and the push with `run`', async () => {
		const { program, calls } = create_recording_program({ kind: 'exited', code: 2 });
		await git_push_to_create(program, 'origin', 'deploy').catch(() => undefined);
		assert.deepEqual(calls[0], {
			method: 'run',
			args: [
				'ls-remote',
				'--exit-code',
				'--heads',
				'--end-of-options',
				'origin',
				'refs/heads/deploy'
			]
		});
		assert.deepEqual(calls[1], {
			method: 'run',
			args: ['push', '-u', '--end-of-options', 'origin', 'deploy']
		});
	});

	test('a local read captures with `out`', async () => {
		const cases: Array<[(git: SpawnProgram) => Promise<unknown>, ReadonlyArray<string>]> = [
			[git_current_branch_name, ['rev-parse', '--abbrev-ref', 'HEAD']],
			[git_check_workspace, ['status', '--porcelain', '-z']],
			[
				(git) => git_local_branch_exists(git, 'main'),
				['show-ref', '--quiet', '--end-of-options', 'refs/heads/main']
			],
			[git_current_commit_hash, ['rev-parse', '--verify', '--quiet', 'HEAD^{commit}']],
			[
				git_current_branch_first_commit_hash,
				['rev-list', '--max-parents=0', '--abbrev-commit', 'HEAD']
			],
			[git_check_setting_pull_rebase, ['config', '--global', 'pull.rebase']]
		];
		for (const [call, args] of cases) {
			const { program, calls } = create_recording_program();
			await call(program);
			assert.deepEqual(calls, [{ method: 'out', args }]);
		}
	});
});

describe('git_pull names beginning with -', () => {
	test('refuses an origin or a branch beginning with - without running git', async () => {
		for (const [origin, branch] of [
			['--upload-pack=touch /tmp/x', 'main'],
			['origin', '--upload-pack=touch /tmp/x']
		] as const) {
			const { program, calls } = create_recording_program();
			let error: unknown;
			try {
				await git_pull(program, origin, branch);
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, Error);
			assert.include(error.message, 'beginning with -');
			assert.deepEqual(calls, []);
		}
	});
});

describe('how a stopped git reads', () => {
	const TIMED_OUT: SpawnResult = { kind: 'timed_out', code: null, signal: 'SIGTERM' };

	test('a stopped local read throws instead of passing partial output as an answer', async () => {
		const { program } = create_recording_program(EXITED_OK, to_output(TIMED_OUT));
		for (const call of [
			() => git_check_workspace(program),
			() => git_current_commit_hash(program),
			() => git_current_branch_name(program)
		]) {
			let error: unknown;
			try {
				await call();
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, Error);
			assert.include(error.message, 'timed out');
		}
	});

	test('a truncated local read throws, an exit of 0 notwithstanding', async () => {
		const { program } = create_recording_program(EXITED_OK, {
			...to_output(EXITED_OK, ' M a.ts\0'),
			truncated: true
		});
		for (const call of [
			() => git_check_workspace(program),
			() => git_current_commit_hash(program),
			() => git_current_branch_name(program)
		]) {
			let error: unknown;
			try {
				await call();
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, Error);
			assert.include(error.message, 'truncated');
			assert.notInclude(error.message, 'code 0');
		}
	});

	test('a stopped fetch fails with the reason', async () => {
		const { program } = create_recording_program(TIMED_OUT);
		let error: unknown;
		try {
			await git_fetch(program, 'origin', 'main');
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, Error);
		assert.include(error.message, 'timed out');
	});

	test('a non-repo reads as no status and no commit', async () => {
		const { program } = create_recording_program(EXITED_OK, {
			...to_output({ kind: 'exited', code: 128 }),
			stderr: 'fatal: not a git repository (or any of the parent directories): .git\n'
		});
		assert.deepEqual(await git_check_workspace(program), {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		});
		assert.strictEqual(await git_current_commit_hash(program), null);
	});

	test('any other non-zero status throws rather than reading as clean', async () => {
		const stderrs = [
			'fatal: .git/index: index file smaller than expected\n',
			"fatal: detected dubious ownership in repository at '/repo'\n",
			'fatal: bad config line 1 in file .git/config\n'
		];
		for (const stderr of stderrs) {
			const { program } = create_recording_program(EXITED_OK, {
				...to_output({ kind: 'exited', code: 128 }),
				stderr
			});
			let error: unknown;
			try {
				await git_check_workspace(program);
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, Error);
			assert.include(error.message, 'git status failed with code 128');
			assert.include(error.message, stderr.trim());
		}
	});

	test('git_remote_branch_exists reads exit 2 as false and other failures as a throw', async () => {
		assert.strictEqual(
			await git_remote_branch_exists(
				create_recording_program({ kind: 'exited', code: 2 }).program,
				'origin',
				'main'
			),
			false
		);
		let error: unknown;
		try {
			await git_remote_branch_exists(
				create_recording_program({ kind: 'exited', code: 128 }).program,
				'origin',
				'main'
			);
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, Error);
		assert.include(error.message, 'code 128');
	});
});

describe('to_git_program', () => {
	test('finds git by its bare name, with the given environment', () => {
		const mock_spawn = create_mock_spawn();
		mock_spawn.available = (name) => name === 'git';
		const git = to_git_program(mock_spawn, { pass_env: GIT_LOCAL_PASS_ENV });
		assert.strictEqual(git.name, 'git');
		assert.deepEqual(mock_spawn.finds, ['git']);
	});

	test('throws a TaskError when git is not on PATH, never looking in node_modules', () => {
		const mock_spawn = create_mock_spawn();
		mock_spawn.available = (name) => name === 'node_modules/.bin/git';
		let error: unknown;
		try {
			to_git_program(mock_spawn);
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'git not found on PATH');
		assert.deepEqual(mock_spawn.finds, ['git']);
	});
});
