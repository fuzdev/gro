import { describe, test, assert } from 'vitest';

import {
	git_check_setting_pull_rebase,
	git_check_workspace,
	git_head_subject,
	git_tag_exists,
	git_checkout,
	git_current_branch_first_commit_hash,
	git_current_branch_name,
	git_current_commit_hash,
	git_delete_local_branch,
	git_fetch,
	git_fetch_force,
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
			out_bytes: () => {
				throw Error('no out_bytes');
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
			[
				(git) => git_fetch_force(git, 'origin', 'deploy'),
				['fetch', '--end-of-options', 'origin', '+deploy:deploy']
			],
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
			[git_head_subject, ['log', '-1', '--format=%s']],
			[
				(git) => git_tag_exists(git, 'v1.0.0'),
				['show-ref', '--quiet', '--end-of-options', 'refs/tags/v1.0.0']
			],
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

describe('git_pull origins beginning with -', () => {
	test('refuses one without running git', async () => {
		const { program, calls } = create_recording_program();
		let error: unknown;
		try {
			await git_pull(program, '--upload-pack=touch /tmp/x', 'main');
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, Error);
		assert.include(error.message, 'beginning with -');
		assert.deepEqual(calls, []);
	});
});

describe('ref names from a caller', () => {
	const REFUSED: ReadonlyArray<string> = [
		':deploy',
		'a:b',
		'ma*',
		'-x',
		'--force',
		'+x',
		'+a:b',
		'refs/heads/x',
		'refs/tags/v1',
		'HEAD',
		'@',
		'@{-1}',
		'a..b',
		'a b',
		'a\x7fb',
		'a~1',
		'a^',
		'a?b',
		'a[b',
		'a\\b',
		'a.lock',
		'a.lock/b',
		'a/.b',
		'.a',
		'a.',
		'/a',
		'a/',
		'a//b',
		''
	];
	const ALLOWED: ReadonlyArray<string> = [
		'main',
		'feature/a.b',
		'a@b',
		'a./b',
		'v1.0.0+build',
		'a+b',
		'a-b',
		'a.locked',
		'x/refs/y',
		'日本語'
	];
	type Call = (git: SpawnProgram, name: string) => Promise<unknown>;
	const CALLS: ReadonlyArray<[string, Call]> = [
		['git_push', (git, name) => git_push(git, 'origin', name)],
		['git_push_to_create', (git, name) => git_push_to_create(git, 'origin', name)],
		['git_pull', (git, name) => git_pull(git, 'origin', name)],
		['git_fetch', (git, name) => git_fetch(git, 'origin', name)],
		['git_fetch_force', (git, name) => git_fetch_force(git, 'origin', name)],
		['git_checkout', (git, name) => git_checkout(git, name)],
		['git_delete_local_branch', (git, name) => git_delete_local_branch(git, name)],
		['git_remote_branch_exists', (git, name) => git_remote_branch_exists(git, 'origin', name)],
		['git_local_branch_exists', (git, name) => git_local_branch_exists(git, name)],
		['git_tag_exists', (git, name) => git_tag_exists(git, name)]
	];

	test.each(CALLS)('%s refuses every non-plain name without running git', async (_, call) => {
		for (const name of REFUSED) {
			const { program, calls } = create_recording_program();
			let error: unknown;
			try {
				await call(program, name);
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, TaskError, `expected a refusal of '${name}'`);
			assert.deepEqual(calls, [], `expected no git call for '${name}'`);
		}
	});

	test.each(CALLS)('%s runs git for a legal name', async (_, call) => {
		for (const name of ALLOWED) {
			const { program, calls } = create_recording_program();
			await call(program, name);
			assert.isAbove(calls.length, 0, `expected a git call for '${name}'`);
		}
	});

	test('the refusal holds for a name `git check-ref-format --branch` accepts', async () => {
		const { program } = create_recording_program();
		let error: unknown;
		try {
			await git_checkout(program, '@');
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'not `HEAD` or `@`');
	});

	test('git_push refuses the current branch on a detached HEAD', async () => {
		const { program, calls } = create_recording_program(EXITED_OK, to_output(EXITED_OK, 'HEAD\n'));
		let error: unknown;
		try {
			await git_push(program, 'origin');
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'detached');
		assert.deepEqual(calls, [{ method: 'out', args: ['rev-parse', '--abbrev-ref', 'HEAD'] }]);
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
