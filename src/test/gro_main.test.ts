import { test, assert, vi, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	gro_main,
	log_cli_error,
	to_exit_code,
	to_grant_flags_unenforced_message
} from '#lib/gro_main.ts';
import { GRO_CLI_GRANTS } from '#lib/gro_builtin_tasks.ts';
import type { ImportModule } from '#lib/modules.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { ReportedError, TaskError } from '#lib/task_error.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';
import { create_test_platform, create_throwing_builtin_host_root } from './test_helpers.ts';

afterEach(() => {
	vi.restoreAllMocks();
	stderr_writes = [];
});

// what a run writes to the root's stderr, the throwing root's other handles left as they are
let stderr_writes: Array<string> = [];

const run = async (
	args: Array<string>,
	version = '0.0.0-test',
	package_json = '{"name": "x"}',
	project: { gro_config?: string; import_module?: ImportModule } = {}
): Promise<void> => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_main_'));
	try {
		writeFileSync(join(dir, 'package.json'), package_json);
		if (project.gro_config !== undefined) {
			writeFileSync(join(dir, 'gro.config.ts'), project.gro_config);
		}
		mkdirSync(join(dir, 'src'));
		await gro_main({
			args,
			fs_paths: await create_project_fs_paths(dir),
			host_root: {
				...create_throwing_builtin_host_root(),
				stderr: {
					write: (data: string) => {
						stderr_writes.push(data);
						return Promise.resolve(true);
					}
				} as never
			},
			import_module: project.import_module ?? import_module_node,
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

test('grant flags warn once on stderr that they confine nothing, and the command runs without them', async () => {
	const warn = vi.spyOn(Logger.prototype, 'warn').mockImplementation(() => undefined);
	const info = vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	await run(['task', '--allow-read=.', '-A']);
	// straight to the root's stderr, never through a logger a log level could quiet
	assert.deepEqual(stderr_writes, [
		to_grant_flags_unenforced_message(['--allow-read=.', '-A']) + '\n'
	]);
	assert.strictEqual(warn.mock.calls.length, 0);
	assert.include(
		to_grant_flags_unenforced_message(['--allow-read=.', '-A']),
		'`--allow-read=.`, `-A`'
	);
	// `gro task` with no path, listing the empty `src/`
	assert.ok(info.mock.calls.some((call) => String(call[0]).includes('No tasks found')));
});

test('no grant flags, no warning', async () => {
	vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	await run(['task']);
	assert.deepEqual(stderr_writes, []);
});

test("a refused task path's hint keeps the grant flags the run was given", async () => {
	const error = vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
	let thrown: unknown;
	try {
		await run(['task', '--allow-read=.', '--x', 'x.task.ts']);
	} catch (e) {
		thrown = e;
	}
	assert.instanceOf(thrown, ReportedError);
	assert.include(String(error.mock.calls[0]![0]), '`gro task --allow-read=. x.task.ts --x`');
});

test('a misspelled grant flag is a usage error before anything runs', async () => {
	const error = vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
	let thrown: unknown;
	try {
		await run(['task', '--allow-reed=.']);
	} catch (e) {
		thrown = e;
	}
	assert.instanceOf(thrown, ReportedError);
	assert.strictEqual(error.mock.calls.length, 1);
	assert.include(String(error.mock.calls[0]![0]), 'is no grant flag');
});

test("an unsynced SvelteKit app's `--version` runs no program, as no sync precedes the command", async () => {
	const info = vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	// the root's spawn throws on any use, and a SvelteKit app never synced would want `svelte-kit`
	await run(
		['--version'],
		'1.2.3-test',
		'{"name": "x", "devDependencies": {"@sveltejs/kit": "*"}}'
	);
	assert.ok(info.mock.calls.some((call) => String(call[0]).includes('1.2.3-test')));
});

test("the CLI's row reads the project and loads its config, frozen", () => {
	assert.deepEqual(JSON.parse(JSON.stringify(GRO_CLI_GRANTS)), {
		fs: { read: ['.'] },
		project_modules: true
	});
	assert.ok(Object.isFrozen(GRO_CLI_GRANTS));
	assert.strictEqual(Object.getPrototypeOf(GRO_CLI_GRANTS), null);
});

test("the project's config loads through the CLI's host, the row's project loader", async () => {
	vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	const import_module = vi.fn<ImportModule>((id) => import_module_node(id));
	await run(['--version'], undefined, undefined, {
		gro_config: 'export default (cfg) => cfg;\n',
		import_module
	});
	assert.ok(import_module.mock.calls.some(([id]) => String(id).endsWith('/gro.config.ts')));
});
