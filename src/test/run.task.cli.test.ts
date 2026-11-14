import { describe, test, expect, vi, afterEach, assert } from 'vitest';

import { invoke_cli, invoke_task } from '#lib/invoke_task.ts';
import { to_raw_rest_args, to_task_args } from '#lib/task_args.ts';
import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { SilentError, TaskError } from '#lib/task.ts';
import { create_mock_config, create_mock_module_graph } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

vi.mock('#lib/gro_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gro_helpers.ts')>()),
	resolve_gro_module_path: vi.fn(() => '/loader.js'),
	spawn_with_loader: vi.fn(async () => ({ kind: 'exited', code: 0 }))
}));

afterEach(() => {
	vi.clearAllMocks();
});

// a file that exists, the script `gro run` is given
const SCRIPT = 'package.json';

// the script args `gro run` spawns for the CLI args after `gro`
const run_cli = async (argv: Array<string>): Promise<Array<string>> => {
	await invoke_cli(
		create_mock_fs_paths(),
		to_task_args(argv),
		await create_mock_config(),
		to_raw_rest_args(argv)
	);
	expect(vi.mocked(spawn_with_loader)).toHaveBeenCalledTimes(1);
	const [, loader_path, path, script_argv] = vi.mocked(spawn_with_loader).mock.calls[0]!;
	expect(loader_path).toBe('/loader.js');
	expect(path).toBe(SCRIPT);
	return [...script_argv];
};

describe('gro run script args', () => {
	test('everything after the path reaches the script verbatim', async () => {
		const rest = [
			'a',
			'--help',
			'--no-cache',
			'--max-warnings',
			'-1',
			'--grep',
			'-foo',
			'--port',
			'0123',
			'--version',
			'1.10',
			'--id',
			'1e3',
			'--x=',
			'--',
			'--literal',
			'--_'
		];
		expect(await run_cli(['run', SCRIPT, ...rest])).toEqual(rest);
	});

	test("a `--` section after the path is the script's, not a forwarded gro section", async () => {
		expect(await run_cli(['run', SCRIPT, '--', 'gro', 'run', '--a'])).toEqual([
			'--',
			'gro',
			'run',
			'--a'
		]);
	});

	test('`invoke_task` serializes named args after the positionals', async () => {
		await invoke_task(
			'run',
			{ _: [SCRIPT, 'a'], port: 3000, watch: true },
			{
				config: await create_mock_config(),
				fs_paths: create_mock_fs_paths(),
				raw_rest_args: [],
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(spawn_with_loader).mock.calls[0]![3]).toEqual([
			'a',
			'--port',
			'3000',
			'--watch'
		]);
	});
});

describe('gro run failure', () => {
	test("a script's exit code is the failure's `exit_code`, which the CLI exits with", async () => {
		vi.mocked(spawn_with_loader).mockResolvedValueOnce({ kind: 'exited', code: 3 });
		const argv = ['run', SCRIPT];
		let error;
		try {
			await invoke_cli(
				create_mock_fs_paths(),
				to_task_args(argv),
				await create_mock_config(),
				to_raw_rest_args(argv)
			);
		} catch (e) {
			error = e;
		}
		assert(error instanceof SilentError);
		assert(error.cause instanceof TaskError);
		expect(error.cause.exit_code).toBe(3);
	});
});
