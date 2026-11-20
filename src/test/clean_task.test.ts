import { describe, test, assert, vi } from 'vitest';

import { run_clean, task as clean_task, type Args } from '#lib/clean.task.ts';
import { clean_fs } from '#lib/clean_fs.ts';
import type { Spawn, SpawnProgram, SpawnResult } from '#lib/spawn.ts';
import { env } from '#lib/env.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import { create_builtin_host, type BuiltinTaskContext } from '#lib/builtin_host.ts';
import { TaskError } from '#lib/task_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

vi.mock('#lib/clean_fs.ts', () => ({ clean_fs: vi.fn() }));

const ARGS: Args = {
	build_dist: false,
	sveltekit: false,
	nodemodules: false,
	git: true,
	git_origin: 'upstream'
};

const create_deps = (result: SpawnResult = { kind: 'exited', code: 0 }) => {
	const git = { name: 'git', run: vi.fn(async () => result) } as unknown as SpawnProgram & {
		run: ReturnType<typeof vi.fn>;
	};
	return {
		git,
		deps: {
			fs_paths: create_mock_fs_paths(),
			programs: { find: vi.fn((_role: 'git_cli') => git as SpawnProgram | null) },
			log: create_mock_logger()
		}
	};
};

const to_error = (promise: Promise<unknown>): Promise<unknown> =>
	promise.then(
		() => undefined,
		(e: unknown) => e
	);

test('`run_clean --build_dist` keeps `.gro/`', async () => {
	const { deps } = create_deps();
	vi.mocked(clean_fs).mockClear();
	await run_clean(
		{ build_dist: true, sveltekit: true, nodemodules: true, git: false, git_origin: 'origin' },
		deps
	);
	assert.deepEqual(vi.mocked(clean_fs).mock.calls[0]![1], {
		gro_dir: false,
		dist_dirs: true,
		sveltekit: true,
		nodemodules: true
	});
});

describe('gro clean --git', () => {
	test('prunes the origin with the git its `host` finds', async () => {
		const { git, deps } = create_deps();
		await run_clean(ARGS, deps);
		assert.deepEqual(
			deps.programs.find.mock.calls.map((c) => c[0]),
			['git_cli']
		);
		assert.deepEqual(git.run.mock.calls[0]![0], [
			'remote',
			'prune',
			'--end-of-options',
			'upstream'
		]);
	});

	test('a failed `git remote prune` is a TaskError', async () => {
		const { deps } = create_deps({ kind: 'exited', code: 2 });
		const error = await to_error(run_clean(ARGS, deps));
		assert.instanceOf(error, TaskError);
		assert.include(error.message, '`git remote prune upstream` failed: code 2');
	});

	test('no git is a TaskError', async () => {
		const { deps } = create_deps();
		deps.programs.find.mockReturnValue(null);
		const error = await to_error(run_clean(ARGS, deps));
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'git not found on PATH');
	});

	test('without `--git` nothing is found', async () => {
		const { deps } = create_deps();
		await run_clean({ ...ARGS, git: false }, deps);
		assert.strictEqual(deps.programs.find.mock.calls.length, 0);
	});
});

test("`clean`'s `run` takes its boundary and git from its `host`", async () => {
	const { git, deps } = create_deps();
	const host = {
		raw_rest_args: [],
		fs_paths: deps.fs_paths,
		programs: { ...deps.programs, program: vi.fn() }
	};
	vi.mocked(clean_fs).mockClear();
	await clean_task.run({
		task_args: ARGS,
		log: deps.log,
		host
	} as unknown as BuiltinTaskContext<Args, typeof clean_task.grants>);
	assert.strictEqual(vi.mocked(clean_fs).mock.calls[0]![0].fs_paths, deps.fs_paths);
	assert.deepEqual(vi.mocked(clean_fs).mock.calls[0]![1], {
		gro_dir: true,
		dist_dirs: true,
		sveltekit: false,
		nodemodules: false
	});
	assert.deepEqual(
		deps.programs.find.mock.calls.map((c) => c[0]),
		['git_cli']
	);
	assert.strictEqual(git.run.mock.calls.length, 1);
});

test("`clean`'s grants give `--git` the whole environment, as it may reach a remote", () => {
	const find = vi.fn(() => null);
	const host = create_builtin_host(
		{
			env,
			spawn: { find, program: vi.fn() } as unknown as Spawn,
			stdout: {} as ProcessOutput,
			stderr: {} as ProcessOutput
		},
		clean_task.grants,
		{
			fs_paths: create_mock_fs_paths(),
			import_module: vi.fn(),
			invoke_project_task: vi.fn(),
			raw_rest_args: []
		}
	);
	assert.deepEqual(Object.keys(host).sort(), ['fs_paths', 'programs', 'raw_rest_args']);
	host.programs.find('git_cli');
	assert.deepEqual(find.mock.calls, [['git', { pass_env: 'all' }]]);
});
