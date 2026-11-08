import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import { chmodSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	find_cli,
	find_local_cli,
	spawn_cli,
	spawn_cli_process,
	spawn_result_is_ok,
	to_spawn_result_message
} from '#lib/cli.ts';
import type { Spawn } from '#lib/spawn.ts';
import { SpawnError, type SpawnErrorKind } from '#lib/spawn_error.ts';
import { is_task_error } from '#lib/task.ts';

import { create_mock_spawn } from './spawn_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

/*

Hermetic: the cwd moves to a sandbox project and `spawn.ts` reloads,
so its project root is the sandbox, and `PATH` holds only the sandbox's `bin`.
`process.chdir` needs Vitest's default `forks` pool.

*/

const write_program = (path: string, body: string): string => {
	writeFileSync(path, `#!/bin/sh\n${body}\n`);
	chmodSync(path, 0o755);
	return path;
};

let base: string;
let project: string;
let path_bin: string;
let local_bin: string;
let cwd: string;
let original_path: string | undefined;
let spawn: Spawn;

beforeEach(async () => {
	cwd = process.cwd();
	original_path = process.env.PATH;
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-cli-')));
	project = join(base, 'project');
	path_bin = join(base, 'bin');
	local_bin = join(project, 'node_modules/.bin');
	mkdirSync(local_bin, { recursive: true });
	mkdirSync(path_bin, { recursive: true });
	process.env.PATH = path_bin;
	process.chdir(project);
	vi.resetModules();
	spawn = (await import('#lib/spawn.ts')).spawn;
});

afterEach(() => {
	process.chdir(cwd);
	process.env.PATH = original_path;
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

describe('find_cli', () => {
	test('a local bin beats `PATH`', () => {
		const local = write_program(join(local_bin, 'tool'), 'exit 0');
		write_program(join(path_bin, 'tool'), 'exit 0');
		const found = find_cli(spawn, 'tool');
		assert.ok(found);
		assert.strictEqual(found.path, local);
	});

	test('`PATH` when there is no local bin', () => {
		const on_path = write_program(join(path_bin, 'tool'), 'exit 0');
		const found = find_cli(spawn, 'tool');
		assert.ok(found);
		assert.strictEqual(found.path, on_path);
		assert.strictEqual(found.name, 'tool');
	});

	test('`null` on a miss', () => {
		assert.strictEqual(find_cli(spawn, 'tool'), null);
	});

	test("a dependency's `.bin/git` doesn't shadow `spawn.program('git')`", () => {
		write_program(join(local_bin, 'git'), 'exit 0');
		const on_path = write_program(join(path_bin, 'git'), 'exit 0');
		assert.strictEqual(spawn.program('git').path, on_path);
	});

	test('an absolute name is refused, as a program name with `/` resolves beneath the project', () => {
		const on_path = write_program(join(path_bin, 'tool'), 'exit 0');
		assert.throws(() => find_cli(spawn, on_path), /not_granted/);
	});

	test('the options fix the program environment', async () => {
		write_program(join(local_bin, 'tool'), 'printf %s "$GRO_CLI_TEST_VAR"');
		process.env.GRO_CLI_TEST_VAR = 'value';
		try {
			assert.strictEqual((await find_cli(spawn, 'tool')!.out([])).stdout, '');
			assert.strictEqual(
				(await find_cli(spawn, 'tool', { pass_env: 'all' })!.out([])).stdout,
				'value'
			);
		} finally {
			delete process.env.GRO_CLI_TEST_VAR;
		}
	});
});

describe('spawn_cli', () => {
	test('logs a local bin by its bare name and runs the args', async () => {
		const mock = create_mock_spawn();
		const log = create_mock_logger();
		const program = find_cli(mock, 'vite')!;
		assert.deepEqual(await spawn_cli(program, ['build'], log), { kind: 'exited', code: 0 });
		assert.deepEqual(mock.finds, ['node_modules/.bin/vite']);
		assert.deepEqual(
			mock.calls.map((c) => [c.method, c.name, c.args]),
			[['run', 'node_modules/.bin/vite', ['build']]]
		);
		assert.include(vi.mocked(log.info).mock.calls[0]![0], ' vite build');
	});

	test('spawn_cli_process starts the program', async () => {
		const mock = create_mock_spawn();
		const child = await spawn_cli_process(find_cli(mock, 'vite')!, ['dev']);
		assert.deepEqual(await child.closed, { kind: 'exited', code: 0 });
		assert.strictEqual(mock.calls[0]!.method, 'start');
	});
});

describe("a program that can't be reached or started", () => {
	const LOCAL_TOOL = 'node_modules/.bin/tool';

	const create_spawn_error = (kind: SpawnErrorKind): SpawnError =>
		new SpawnError(kind, 'tool', `${kind}: the message`);

	const capture = async (fn: () => unknown): Promise<unknown> => {
		try {
			await fn();
		} catch (error) {
			return error;
		}
		return assert.fail('expected a throw');
	};

	const assert_task_error = (thrown: unknown, spawn_error: SpawnError): void => {
		assert.ok(is_task_error(thrown));
		assert.strictEqual(thrown.message, spawn_error.message);
		assert.strictEqual(thrown.cause, spawn_error);
	};

	test('find_local_cli throws a `TaskError`', async () => {
		const mock = create_mock_spawn();
		const error = create_spawn_error('not_executable');
		mock.find_error = () => error;
		assert_task_error(await capture(() => find_local_cli(mock, 'tool')), error);
	});

	test('find_cli throws a `TaskError` for the local bin', async () => {
		const mock = create_mock_spawn();
		const error = create_spawn_error('not_executable');
		mock.find_error = (name) => (name === LOCAL_TOOL ? error : undefined);
		assert_task_error(await capture(() => find_cli(mock, 'tool')), error);
	});

	test('find_cli throws a `TaskError` for the `PATH` lookup', async () => {
		const mock = create_mock_spawn();
		const error = create_spawn_error('not_granted');
		mock.available = () => false;
		mock.find_error = (name) => (name === 'tool' ? error : undefined);
		assert_task_error(await capture(() => find_cli(mock, 'tool')), error);
		assert.deepEqual(mock.finds, [LOCAL_TOOL, 'tool']);
	});

	test('find_cli is still `null` on a miss', () => {
		const mock = create_mock_spawn();
		mock.available = () => false;
		assert.strictEqual(find_cli(mock, 'tool'), null);
	});

	test('spawn_cli rejects with a `TaskError`', async () => {
		const mock = create_mock_spawn();
		const error = create_spawn_error('launch_failed');
		mock.run_error = () => error;
		assert_task_error(await capture(() => spawn_cli(find_cli(mock, 'tool')!)), error);
	});

	test('spawn_cli_process rejects with a `TaskError`', async () => {
		const mock = create_mock_spawn();
		const error = create_spawn_error('launch_failed');
		mock.start_error = () => error;
		assert_task_error(await capture(() => spawn_cli_process(find_cli(mock, 'tool')!)), error);
	});

	test('any other error passes through unchanged', async () => {
		const error = Error('not a spawn error');
		const finding = create_mock_spawn();
		finding.find_error = () => error;
		assert.strictEqual(await capture(() => find_cli(finding, 'tool')), error);
		assert.strictEqual(await capture(() => find_local_cli(finding, 'tool')), error);

		const running = create_mock_spawn();
		running.run_error = () => error;
		running.start_error = () => error;
		const program = find_cli(running, 'tool')!;
		assert.strictEqual(await capture(() => spawn_cli(program)), error);
		assert.strictEqual(await capture(() => spawn_cli_process(program)), error);
	});
});

describe('spawn results', () => {
	test('only an exit with 0 is ok', () => {
		assert.ok(spawn_result_is_ok({ kind: 'exited', code: 0 }));
		assert.ok(!spawn_result_is_ok({ kind: 'exited', code: 1 }));
		assert.ok(!spawn_result_is_ok({ kind: 'signaled', signal: 'SIGKILL' }));
		assert.ok(!spawn_result_is_ok({ kind: 'timed_out', code: 0, signal: null }));
		assert.ok(!spawn_result_is_ok({ kind: 'aborted', code: 0, signal: null }));
	});

	test('messages name each kind', () => {
		assert.strictEqual(to_spawn_result_message({ kind: 'exited', code: 1 }), 'code 1');
		assert.strictEqual(
			to_spawn_result_message({ kind: 'signaled', signal: 'SIGTERM' }),
			'signal SIGTERM'
		);
		assert.strictEqual(
			to_spawn_result_message({ kind: 'timed_out', code: null, signal: 'SIGTERM' }),
			'timed out'
		);
		assert.strictEqual(
			to_spawn_result_message({ kind: 'aborted', code: null, signal: 'SIGTERM' }),
			'aborted'
		);
	});
});
