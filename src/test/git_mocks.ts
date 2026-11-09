import { vi, type Mock } from 'vitest';

import type { SpawnCallOptions, SpawnOutput, SpawnProgram, SpawnResult } from '#lib/spawn.ts';

import { create_mock_spawn, type MockSpawn, type MockSpawnCall } from './spawn_mocks.ts';

/*

The real `#lib/git.ts` runs over a mock program in these suites, so its argv is what's asserted,
while a suite scripts git by what each operation answers: `git_ops.git_fetch` answers a real
`git fetch` the program received, its arguments read back out of that argv. A `git` argv the
table doesn't know throws, so an argv change reaches a test rather than a silent pass. Commands
with no table entry, the ones the deploy task composes itself, forward to `git_command` and
`git_command_out` in the shape `mock_spawn_results` scripts.

*/

/**
 * The cwd of a call as it was passed by path, the argument the git functions take it as.
 */
export const to_mock_git_options = (
	options: SpawnCallOptions | undefined
): { cwd: string } | undefined =>
	options?.cwd === undefined ? undefined : { cwd: options.cwd.path };

/**
 * `args` with the trailing `undefined`s dropped, so a mocked operation is called as its caller
 * called it, and `toHaveBeenCalledWith` sees no extra argument.
 */
const to_call_args = (...args: Array<unknown>): Array<any> => {
	const end = args.length - 1;
	return end >= 0 && args[end] === undefined ? to_call_args(...args.slice(0, end)) : args;
};

/**
 * What the mocked git operations answer, a stand-in for each function of `git.ts`
 * that took a program, in that function's own arguments and return shape.
 */
export interface MockGitOps {
	git_current_branch_name: Mock<(...args: Array<any>) => Promise<string>>;
	git_remote_branch_exists: Mock<(...args: Array<any>) => Promise<boolean>>;
	git_local_branch_exists: Mock<(...args: Array<any>) => Promise<boolean>>;
	git_tag_exists: Mock<(...args: Array<any>) => Promise<boolean>>;
	git_head_subject: Mock<(...args: Array<any>) => Promise<string | null>>;
	git_check_clean_workspace: Mock<(...args: Array<any>) => Promise<string | null>>;
	git_fetch: Mock<(...args: Array<any>) => Promise<void>>;
	git_checkout: Mock<(...args: Array<any>) => Promise<string | null>>;
	git_pull: Mock<(...args: Array<any>) => Promise<void>>;
	git_push_to_create: Mock<(...args: Array<any>) => Promise<void>>;
	git_delete_local_branch: Mock<(...args: Array<any>) => Promise<void>>;
	git_current_commit_hash: Mock<(...args: Array<any>) => Promise<string | null>>;
	git_current_branch_first_commit_hash: Mock<(...args: Array<any>) => Promise<string>>;
	git_check_setting_pull_rebase: Mock<(...args: Array<any>) => Promise<boolean>>;
	/**
	 * A command with no operation of its own, as `spawn` was: `(command, args, options)`,
	 * resolving to a `SpawnResult`.
	 */
	git_command: Mock<(...args: Array<any>) => Promise<SpawnResult>>;
	/**
	 * The same for a captured command, as `spawn_out` was, resolving to its `{result, stdout, stderr}`.
	 */
	git_command_out: Mock<
		(...args: Array<any>) => Promise<{ result: SpawnResult; stdout: string; stderr: string }>
	>;
}

/**
 * The operations a suite scripts. `vi.resetAllMocks` restores each to answering `undefined`.
 */
export const git_ops: MockGitOps = {
	git_current_branch_name: vi.fn(),
	git_remote_branch_exists: vi.fn(),
	git_local_branch_exists: vi.fn(),
	git_check_clean_workspace: vi.fn(),
	git_fetch: vi.fn(),
	git_checkout: vi.fn(),
	git_pull: vi.fn(),
	git_push_to_create: vi.fn(),
	git_delete_local_branch: vi.fn(),
	git_current_commit_hash: vi.fn(),
	git_tag_exists: vi.fn(),
	git_head_subject: vi.fn(),
	git_current_branch_first_commit_hash: vi.fn(),
	git_check_setting_pull_rebase: vi.fn(),
	git_command: vi.fn(),
	git_command_out: vi.fn()
};

const to_output = (stdout = '', code = 0): SpawnOutput => ({
	kind: 'exited',
	code,
	stdout,
	stderr: '',
	truncated: false
});

/**
 * A status message from a mocked `git_check_clean_workspace` as `git status --porcelain -z` output:
 * a message holding a NUL is used as it is, for a test that needs a particular porcelain,
 * and any other dirty message is one modified file.
 */
const to_status_output = (message: string | null | undefined): string =>
	!message ? '' : message.includes('\0') ? message : ' M src/changed.ts\0';

const after_end_of_options = (args: ReadonlyArray<string>): ReadonlyArray<string> =>
	args.slice(args.indexOf('--end-of-options') + 1);

const from_exit_code_answer = (answer: unknown, false_code: number): SpawnOutput =>
	to_output('', answer === false ? false_code : 0);

const forward = (call: MockSpawnCall, ops: MockGitOps): Promise<SpawnOutput> =>
	call.method === 'out'
		? ops
				.git_command_out(...to_call_args(call.name, call.args, to_mock_git_options(call.options)))
				.then(({ result, stdout, stderr }) => ({ ...result, stdout, stderr, truncated: false }))
		: ops
				.git_command(...to_call_args(call.name, call.args, to_mock_git_options(call.options)))
				.then((result) => ({ ...result, stdout: '', stderr: '', truncated: false }));

/**
 * Answers a `git` call from the table.
 */
export const answer_git_call = async (
	call: MockSpawnCall,
	ops: MockGitOps = git_ops
): Promise<SpawnOutput> => {
	const { args, options } = call;
	const opts = to_mock_git_options(options);
	const [command] = args;
	const rest = after_end_of_options(args);
	switch (command) {
		case 'ls-remote':
			return from_exit_code_answer(
				await ops.git_remote_branch_exists(
					...to_call_args(rest[0], rest[1]?.replace('refs/heads/', ''), opts)
				),
				2
			);
		case 'fetch':
			await ops.git_fetch(...to_call_args(rest[0], rest[1], opts));
			return to_output();
		case 'pull':
			await ops.git_pull(...to_call_args(args[1], args[2], opts));
			return to_output();
		case 'checkout':
			if (args[1] === '--orphan') return forward(call, ops);
			await ops.git_checkout(...to_call_args(rest[0], opts));
			return to_output();
		case 'push':
			if (args.includes('--end-of-options') && args[1] !== '-f' && args[1] !== '--follow-tags') {
				await ops.git_push_to_create(...to_call_args(rest[0], rest[1], opts));
				return to_output();
			}
			return forward(call, ops);
		case 'branch':
			await ops.git_delete_local_branch(...to_call_args(rest[0], opts));
			return to_output();
		case 'status':
			return to_output(
				to_status_output(await ops.git_check_clean_workspace(...to_call_args(opts)))
			);
		case 'config':
			if (args[1] === '--global') {
				return to_output((await ops.git_check_setting_pull_rebase()) ? 'true\n' : '');
			}
			return forward(call, ops);
		case 'log':
			return to_output(((await ops.git_head_subject(...to_call_args(opts))) ?? '') + '\n');
		case 'show-ref':
			if (args[1] === '--quiet' && rest[0]!.startsWith('refs/tags/')) {
				return from_exit_code_answer(
					await ops.git_tag_exists(...to_call_args(rest[0]!.replace('refs/tags/', ''), opts)),
					1
				);
			}
			if (args[1] === '--quiet') {
				return from_exit_code_answer(
					await ops.git_local_branch_exists(
						...to_call_args(rest[0]!.replace('refs/heads/', ''), opts)
					),
					1
				);
			}
			return forward(call, ops);
		case 'rev-parse':
			if (args[1] === '--abbrev-ref') {
				return to_output(
					((await ops.git_current_branch_name(...to_call_args(opts))) ?? 'mock-branch') + '\n'
				);
			}
			if (args[1] === '--verify') {
				const hash = await ops.git_current_commit_hash(...to_call_args(opts));
				return to_output(hash ? hash + '\n' : '', hash ? 0 : 1);
			}
			return forward(call, ops);
		case 'rev-list':
			return to_output(
				((await ops.git_current_branch_first_commit_hash(...to_call_args(opts))) ?? '') + '\n'
			);
		case 'clone':
		case 'remote':
		case 'ls-files':
		case 'diff':
		case 'reset':
		case 'add':
		case 'commit':
		case 'rm':
			return forward(call, ops);
		default:
			throw Error(`the mock git table doesn't know: git ${args.join(' ')}`);
	}
};

/**
 * Routes a mock spawn's `git` calls to the table: a suite's `git_ops` answer them,
 * and any other program runs as the mock's `result` says.
 */
export const install_mock_git = (mock_spawn: MockSpawn, ops: MockGitOps = git_ops): void => {
	mock_spawn.available = (name) => name === 'git' || name.startsWith('node_modules/.bin/');
	const fallback: SpawnResult = { kind: 'exited', code: 0 };
	mock_spawn.result = (call) => (call.name === 'git' ? answer_git_call(call, ops) : fallback);
	mock_spawn.out_result = (call) => answer_git_call(call, ops);
};

/**
 * A `git` `SpawnProgram` answered from the table, for library code taking one directly.
 */
export const create_mock_git_program = (ops: MockGitOps = git_ops): SpawnProgram => ({
	name: 'git',
	path: '/mock/git',
	run: (args, options) =>
		answer_git_call({ method: 'run', name: 'git', args, options, program_options: undefined }, ops),
	out: (args, options) =>
		answer_git_call({ method: 'out', name: 'git', args, options, program_options: undefined }, ops),
	out_bytes: () => {
		throw Error('mock git program has no out_bytes');
	},
	start: () => {
		throw Error('mock git program has no start');
	}
});

/**
 * The spawn host module with `spawn` a mock whose `git` is answered from the table, for a test of a task:
 * `vi.mock('#lib/spawn.ts', async () => (await import('./git_mocks.ts')).create_mock_git_spawn_module())`.
 */
export const create_mock_git_spawn_module = (): typeof import('#lib/spawn.ts') => {
	const spawn = create_mock_spawn();
	install_mock_git(spawn);
	return { spawn };
};
