import { describe, test, expect, vi, afterEach, assert } from 'vitest';

import { invoke_cli, invoke_task } from '#lib/invoke_task.ts';
import { to_task_args } from '#lib/args.ts';
import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { SilentError, TaskError } from '#lib/task.ts';
import {
	create_mock_config,
	create_mock_module_graph,
	create_mock_watcher
} from './test_helpers.ts';

vi.mock('#lib/gro_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gro_helpers.ts')>()),
	resolve_gro_module_path: vi.fn(() => '/loader.js'),
	spawn_with_loader: vi.fn(async () => ({ ok: true, code: 0, signal: null }))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

// a file that exists, the script `gro run` is given
const SCRIPT = 'package.json';

// the script args `gro run` spawns for the CLI args after `gro`
const run_cli = async (argv: Array<string>): Promise<Array<string>> => {
	process.argv = ['node', 'gro', ...argv];
	await invoke_cli(to_task_args(), await create_mock_config());
	expect(vi.mocked(spawn_with_loader)).toHaveBeenCalledTimes(1);
	const [loader_path, path, script_argv] = vi.mocked(spawn_with_loader).mock.calls[0]!;
	expect(loader_path).toBe('/loader.js');
	expect(path).toBe(SCRIPT);
	return script_argv;
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
		process.argv = ['node', 'gro', 'x'];
		await invoke_task(
			'run',
			{ _: [SCRIPT, 'a'], port: 3000, watch: true },
			{
				config: await create_mock_config(),
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
			}
		);
		expect(vi.mocked(spawn_with_loader).mock.calls[0]![2]).toEqual([
			'a',
			'--port',
			'3000',
			'--watch'
		]);
	});
});

describe('gro run failure', () => {
	test("a script's exit code is the failure's `exit_code`, which the CLI exits with", async () => {
		vi.mocked(spawn_with_loader).mockResolvedValueOnce({
			ok: false,
			kind: 'exited',
			code: 3,
			child: null as any
		});
		process.argv = ['node', 'gro', 'run', SCRIPT];
		let error;
		try {
			await invoke_cli(to_task_args(), await create_mock_config());
		} catch (e) {
			error = e;
		}
		assert(error instanceof SilentError);
		assert(error.cause instanceof TaskError);
		expect(error.cause.exit_code).toBe(3);
	});
});
