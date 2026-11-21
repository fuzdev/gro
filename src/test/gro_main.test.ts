import { test, assert, vi, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { gro_main, log_cli_error, to_exit_code } from '#lib/gro_main.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { ReportedError, TaskError } from '#lib/task_error.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';
import { create_test_platform, create_throwing_builtin_host_root } from './test_helpers.ts';

afterEach(() => {
	vi.restoreAllMocks();
});

const run = async (args: Array<string>, version = '0.0.0-test'): Promise<void> => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_main_'));
	try {
		writeFileSync(join(dir, 'package.json'), '{"name": "x"}');
		await gro_main({
			args,
			fs_paths: await create_project_fs_paths(dir),
			host_root: create_throwing_builtin_host_root(),
			import_module: import_module_node,
			platform: { ...create_test_platform(), gro: { version, dir: null } }
		});
	} finally {
		rmSync(dir, { recursive: true });
	}
};

test("`--version` prints the version of the Gro the root's platform names", async () => {
	const info = vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	await run(['--version'], '9.8.7-test');
	assert.ok(info.mock.calls.some((call) => String(call[0]).includes('9.8.7-test')));
});

test('a usage error is reported once, with no stack, and thrown as reported', async () => {
	const error = vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
	let thrown: unknown;
	try {
		await run(['task', '--a', 'x.task.ts']);
	} catch (e) {
		thrown = e;
	}
	assert.instanceOf(thrown, ReportedError);
	assert.strictEqual(error.mock.calls.length, 1);
	assert.include(String(error.mock.calls[0]![0]), 'takes no flags before the path');
});

test('an unknown builtin is reported and thrown as reported', async () => {
	vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
	vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	let thrown: unknown;
	try {
		await run(['no_such_builtin']);
	} catch (e) {
		thrown = e;
	}
	assert.instanceOf(thrown, ReportedError);
});

test("to_exit_code reads a task error's code, its own or its cause's, else 1", () => {
	assert.strictEqual(to_exit_code(new TaskError('x', { exit_code: 3 })), 3);
	assert.strictEqual(to_exit_code(new ReportedError(undefined, { cause: { exit_code: 7 } })), 7);
	assert.strictEqual(to_exit_code(Object.assign(Error('x'), { exit_code: 256 })), 1);
	assert.strictEqual(to_exit_code(Object.assign(Error('x'), { exit_code: 1.5 })), 1);
	assert.strictEqual(to_exit_code(undefined), 1);
	assert.strictEqual(to_exit_code(Error('x')), 1);
});

test('log_cli_error logs a task error without its stack and a reported one not at all', () => {
	const error = vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
	log_cli_error(new ReportedError(), 'origin');
	assert.strictEqual(error.mock.calls.length, 0);
	log_cli_error(new TaskError('the message'), 'origin');
	assert.strictEqual(error.mock.calls.length, 1);
	assert.include(String(error.mock.calls[0]![0]), 'the message');
	assert.notInclude(String(error.mock.calls[0]![0]), 'at ');
	log_cli_error(null, 'origin');
	assert.include(String(error.mock.calls[1]![0]), 'Unknown error: null');
	log_cli_error(Error('boom'), 'origin');
	assert.include(String(error.mock.calls[2]![0]), 'boom');
});
