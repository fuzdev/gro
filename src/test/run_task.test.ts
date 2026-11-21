import { test, expect, assert, vi } from 'vitest';
import { resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { styleText } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { spawn } from '#lib/spawn.ts';
import { stdout } from '#lib/stdout.ts';
import { stderr } from '#lib/stderr.ts';
import { run_task, type RunTaskOptions } from '#lib/run_task.ts';
import type { Task, TaskContext } from '#lib/task.ts';
import { ReportedError, TaskError } from '#lib/task_error.ts';
import { task as run_task_task } from '#lib/run.task.ts';
import { FsError } from '#lib/fs_error.ts';
import { NetError } from '#lib/net_error.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { EnvError, is_env_error } from '#lib/env_error.ts';
import { StdioError } from '#lib/stdio_error.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { env } from '#lib/env.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from '#lib/gro_builtin_tasks.ts';

import {
	TEST_TIMEOUT_MD,
	LOADER_SOURCE_PATH,
	create_mock_config,
	create_mock_module_graph,
	create_mock_logger,
	create_mock_timings,
	create_throwing_builtin_host_root,
	assert_throws_matching,
	create_test_platform
} from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

const node = spawn.program('node', { pass_env: 'all' });

test(
	'run_task passes args and returns output',
	async () => {
		const testScript = resolve('src/test/fixtures/test_run_task_basic.ts');

		// Log test script path for debugging
		// eslint-disable-next-line no-console
		console.log('Test script path:', testScript);
		// eslint-disable-next-line no-console
		console.log('Test script exists:', existsSync(testScript));

		const result = await spawn_with_loader(node, LOADER_SOURCE_PATH, testScript, [], undefined);

		expect(result).toEqual({ kind: 'exited', code: 0 });
	},
	TEST_TIMEOUT_MD
);

test(
	'run_task invokes sub tasks',
	async () => {
		const testScript = resolve('src/test/fixtures/test_run_task_invoke.ts');

		// Log test script path for debugging
		// eslint-disable-next-line no-console
		console.log('Test script path:', testScript);
		// eslint-disable-next-line no-console
		console.log('Test script exists:', existsSync(testScript));

		const result = await spawn_with_loader(node, LOADER_SOURCE_PATH, testScript, [], undefined);

		expect(result).toEqual({ kind: 'exited', code: 0 });
	},
	TEST_TIMEOUT_MD
);

test(
	'run_task handles failing tasks',
	async () => {
		const testScript = resolve('src/test/fixtures/test_run_task_failure.ts');

		// Log test script path for debugging
		// eslint-disable-next-line no-console
		console.log('Test script path:', testScript);
		// eslint-disable-next-line no-console
		console.log('Test script exists:', existsSync(testScript));

		const result = await spawn_with_loader(node, LOADER_SOURCE_PATH, testScript, [], undefined);

		expect(result).toEqual({ kind: 'exited', code: 0 });
	},
	TEST_TIMEOUT_MD
);

const create_mock_run_task_options = async (): Promise<RunTaskOptions> => ({
	invoke_task: vi.fn(),
	config: await create_mock_config(),
	fs_paths: create_mock_fs_paths(),
	raw_rest_args: [],
	import_module: import_module_node,
	host_root: create_throwing_builtin_host_root(),
	platform: create_test_platform(),
	implicit_rest_args: [],
	grants: null,
	graph: create_mock_module_graph(),
	log: create_mock_logger(),
	timings: create_mock_timings()
});

test('run_task returns a `TaskError` failure listing each issue when args fail validation', async () => {
	const run = vi.fn();
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					Args: s.object({ name: s.string, count: s.number }),
					run
				}
			}
		},
		{ name: 1, count: 'x' },
		await create_mock_run_task_options()
	);
	assert(!result.ok);
	assert(result.error instanceof TaskError);
	const message =
		"Failed task args validation for task 'greet':\n" +
		'name: expected string, received number\n' +
		'count: expected number, received string';
	assert.strictEqual(result.error.message, message);
	assert.include(result.reason, message);
	assert.strictEqual(run.mock.calls.length, 0);
});

test('run_task prints the issue of each bad forwarded arg of `gro run`', async () => {
	const result = await run_task(
		{ name: 'run', id: '/p/src/lib/run.task.ts', mod: { task: run_task_task as Task<any> } },
		{ _: ['script.ts'], x: { a: 1 }, y: null } as any,
		await create_mock_run_task_options()
	);
	assert(!result.ok);
	assert(result.error instanceof TaskError);
	assert.strictEqual(
		result.error.message,
		"Failed task args validation for task 'run':\n" +
			'x: expected string | number | boolean | array, received object\n' +
			'y: expected string | number | boolean | array, received null'
	);
});

test('run_task reports a failed nested `invoke_task` as a failure, not an unexpected error', async () => {
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: async (ctx) => {
						const task_name: string = 'former_project_task'; // widened, not a registered task
						await ctx.invoke_task(task_name);
					}
				}
			}
		},
		{},
		{
			...(await create_mock_run_task_options()),
			invoke_task: () => Promise.reject(new ReportedError()) // `invoke_task` already logged why
		}
	);
	assert(!result.ok);
	assert(result.error instanceof ReportedError);
	assert.include(result.reason, 'failed.');
	assert.notInclude(result.reason, 'Unexpected error');
});

test('run_task reports a `TaskError` subclass by its message, not as an unexpected error', async () => {
	class SubTaskError extends TaskError {}
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: () => {
						throw new SubTaskError('known failure');
					}
				}
			}
		},
		{},
		await create_mock_run_task_options()
	);
	assert(!result.ok);
	assert.include(result.reason, 'known failure');
	assert.notInclude(result.reason, 'Unexpected error');
});

test('run_task reports a host module error as the task failing, without the reinstall hint', async () => {
	for (const error of [
		new FsError('not_found', '/p/missing.txt'),
		new SpawnError('not_found', 'missing_program'),
		new NetError('not_granted', 'example.com:443'),
		new EnvError('not_granted', 'SECRET'),
		new StdioError('io_error', 'stdout')
	]) {
		const result = await run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: {
					task: {
						run: () => {
							throw error;
						}
					}
				}
			},
			{},
			await create_mock_run_task_options()
		);
		assert(!result.ok);
		assert.strictEqual(result.error, error);
		assert.strictEqual(result.reason, `Task ${styleText('cyan', 'greet')} failed.`, error.name);
	}
});

test('run_task reports any other error as unexpected, with the reinstall hint', async () => {
	const error = new Error('broken');
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: () => {
						throw error;
					}
				}
			}
		},
		{},
		await create_mock_run_task_options()
	);
	assert(!result.ok);
	assert.strictEqual(result.error, error);
	assert.include(result.reason, 'Unexpected error running task');
	assert.include(result.reason, 'install` and `gro clean`');
});

test('run_task passes the graph to the task, and it, `fs_paths`, and `host_root` to the tasks it invokes', async () => {
	const graph = create_mock_module_graph();
	const fs_paths = create_mock_fs_paths();
	const host_root = create_throwing_builtin_host_root();
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const config = await create_mock_config();
	const other_config = await create_mock_config({ search_filters: [] });
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: async (ctx) => {
						const task_name: string = 'other'; // widened, not a registered task
						await ctx.invoke_task(task_name);
						await ctx.invoke_task(task_name, undefined, { config: other_config });
						return ctx;
					}
				}
			}
		},
		{},
		{
			invoke_task,
			config,
			fs_paths,
			raw_rest_args: ['--', 'x'],
			import_module: import_module_node,
			host_root,
			platform: create_test_platform(),
			implicit_rest_args: ['x'],
			grants: null,
			graph,
			log: create_mock_logger(),
			timings: create_mock_timings()
		}
	);
	assert(result.ok);
	const ctx = result.output as TaskContext;
	assert.strictEqual(ctx.graph, graph);
	assert.ok(!('fs_paths' in ctx));
	assert.deepEqual(ctx.raw_rest_args, ['--', 'x']);
	assert.ok(!('import_module' in ctx));
	assert.ok(!('host_root' in ctx));
	assert.ok(!('host' in ctx));
	expect(invoke_task).toHaveBeenNthCalledWith(1, 'other', undefined, {
		config,
		fs_paths,
		raw_rest_args: ['--', 'x'],
		import_module: import_module_node,
		host_root,
		platform: create_test_platform(),
		graph,
		timings: expect.anything(),
		parent_log: expect.anything()
	});
	expect(invoke_task).toHaveBeenNthCalledWith(2, 'other', undefined, {
		config: other_config,
		fs_paths,
		raw_rest_args: ['--', 'x'],
		import_module: import_module_node,
		host_root,
		platform: create_test_platform(),
		graph,
		timings: expect.anything(),
		parent_log: expect.anything()
	});
});

test('run_task gives a builtin with grants a `host` cut by them, and a project task none', async () => {
	const fs_paths = create_mock_fs_paths();
	const run = (ctx: TaskContext) => ctx;
	const options = {
		...(await create_mock_run_task_options()),
		fs_paths,
		raw_rest_args: ['--', 'x'],
		host_root: { env, spawn, stdout, stderr },
		platform: create_test_platform(),
		implicit_rest_args: ['x']
	};
	const project = await run_task(
		{ name: 'src/lib/check.task.ts', id: '/p/src/lib/check.task.ts', mod: { task: { run } } },
		{},
		options
	);
	assert(project.ok);
	assert.ok(!('host' in (project.output as TaskContext)));
	// the forwarded args are data on every task's context, a frozen copy
	const { raw_rest_args } = project.output as TaskContext;
	assert.deepEqual(raw_rest_args, ['--', 'x']);
	assert.notStrictEqual(raw_rest_args, options.raw_rest_args);
	assert.ok(Object.isFrozen(raw_rest_args));
	const builtin = await run_task(
		{ name: 'greet', id: '/p/src/lib/greet.task.ts', mod: { task: { run } } },
		{},
		{ ...options, grants: { fs: { read: ['.'] }, env: ['CI'] } }
	);
	assert(builtin.ok);
	const { host } = builtin.output as BuiltinTaskContext<
		object,
		{ fs: { read: ['.'] }; env: ['CI'] }
	>;
	assert.deepEqual(Object.keys(host), ['fs_paths', 'env']);
	assert.strictEqual(host.fs_paths, fs_paths);
	assert.deepEqual((builtin.output as TaskContext).raw_rest_args, ['--', 'x']);
	assert_throws_matching(
		() => host.env.get('HOME'),
		(error: unknown) => is_env_error(error, 'not_granted')
	);
});

test('run_task gives every task its own frozen copy of `implicit_rest_args`', async () => {
	const implicit_rest_args = ['--watch'];
	const options = { ...(await create_mock_run_task_options()), implicit_rest_args };
	const run = (ctx: TaskContext) => ctx;
	for (const grants of [null, { fs: { read: ['.'] } }]) {
		const result = await run_task(
			{ name: 'greet', id: '/p/src/lib/greet.task.ts', mod: { task: { run } } },
			{},
			{ ...options, grants }
		);
		assert(result.ok);
		const ctx = result.output as TaskContext;
		assert.deepEqual(ctx.implicit_rest_args, ['--watch']);
		assert.notStrictEqual(ctx.implicit_rest_args, implicit_rest_args);
		assert.ok(Object.isFrozen(ctx.implicit_rest_args));
	}
	implicit_rest_args.push('x');
	const result = await run_task(
		{ name: 'greet', id: '/p/src/lib/greet.task.ts', mod: { task: { run } } },
		{},
		await create_mock_run_task_options()
	);
	assert(result.ok);
	assert.deepEqual((result.output as TaskContext).implicit_rest_args, []);
});

test("a task's `invoke_task` passes `implicit_rest_args` only when given them", async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const options = {
		...(await create_mock_run_task_options()),
		invoke_task,
		implicit_rest_args: ['--watch']
	};
	for (const grants of [null, { invokes: { test: {} } }]) {
		invoke_task.mockClear();
		const result = await run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: {
					task: {
						run: async (ctx) => {
							await ctx.invoke_task('test');
							await ctx.invoke_task('test', undefined, { implicit_rest_args: ['--bail', '1'] });
						}
					}
				}
			},
			{},
			{ ...options, grants }
		);
		assert(result.ok);
		const passed = invoke_task.mock.calls.map((call) => call[2] as Record<string, unknown>);
		assert.ok(!('implicit_rest_args' in passed[0]!));
		assert.deepEqual(passed[1]!.implicit_rest_args, ['--bail', '1']);
	}
});

test('run_task gives a builtin granted `project_modules` its own project loader, which lends no invoke', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const import_module = vi.fn();
	const options = { ...(await create_mock_run_task_options()), invoke_task, import_module };
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: async (ctx) => {
						const { host } = ctx as unknown as BuiltinTaskContext<
							object,
							{ project_modules: true }
						>;
						assert.deepEqual(Object.keys(host), ['import_module']);
						await ctx.invoke_task('lint');
						return host;
					}
				}
			}
		},
		{},
		{ ...options, grants: { project_modules: true } }
	);
	// its `invokes` names none, and `project_modules` adds none
	assert(!result.ok);
	assert.include(result.reason, "which its grants don't name");
	expect(invoke_task).not.toHaveBeenCalled();
});

test('run_task refuses a builtin invoking a task its grants do not name', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: async (ctx) => {
						await ctx.invoke_task('gen');
						await ctx.invoke_task('lint');
					}
				}
			}
		},
		{},
		{ ...(await create_mock_run_task_options()), invoke_task, grants: { invokes: { gen: {} } } }
	);
	assert(!result.ok);
	assert.instanceOf(result.error, TaskError);
	assert.include(result.reason, "invokes `lint`, which its grants don't name");
	assert.deepEqual(
		invoke_task.mock.calls.map((c) => c[0]),
		['gen']
	);
});

test('run_task refuses an invoke its grants mark `when` an arg the task was run without', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const options = await create_mock_run_task_options();
	const run_with = (build: boolean) =>
		run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: {
					task: {
						Args: s.object({ build: s.with_default(s.boolean, false) }),
						run: async (ctx) => {
							await ctx.invoke_task('gen');
							await ctx.invoke_task('build');
						}
					}
				}
			},
			{ build },
			{
				...options,
				invoke_task,
				grants: { invokes: { gen: {}, build: { when: 'build' } } }
			}
		);
	const refused = await run_with(false);
	assert(!refused.ok);
	assert.instanceOf(refused.error, TaskError);
	assert.include(
		refused.reason,
		"invokes `build` with its `build` arg unset, and its grants invoke `build` only when it's set"
	);
	assert.deepEqual(
		invoke_task.mock.calls.map((c) => c[0]),
		['gen']
	);
	invoke_task.mockClear();
	const ran = await run_with(true);
	assert(ran.ok);
	assert.deepEqual(
		invoke_task.mock.calls.map((c) => c[0]),
		['gen', 'build']
	);
});

test("run_task reads a `when` arg as the task's own, never one added to `Object.prototype`", async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const polluted = Object.prototype as unknown as Record<string, unknown>;
	polluted.build = true;
	try {
		const result = await run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: { task: { run: (ctx) => ctx.invoke_task('build') } }
			},
			{},
			{
				...(await create_mock_run_task_options()),
				invoke_task,
				grants: { invokes: { build: { when: 'build' } } }
			}
		);
		assert(!result.ok);
		assert.include(result.reason, 'with its `build` arg unset');
	} finally {
		delete polluted.build;
	}
	assert.strictEqual(invoke_task.mock.calls.length, 0);
});

test('run_task grants no invoke by a name added to `Object.prototype`', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const polluted = Object.prototype as unknown as Record<string, unknown>;
	polluted.lint = {};
	try {
		const result = await run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: { task: { run: (ctx) => ctx.invoke_task('lint') } }
			},
			{},
			{ ...(await create_mock_run_task_options()), invoke_task, grants: { invokes: {} } }
		);
		assert(!result.ok);
		assert.include(result.reason, "invokes `lint`, which its grants don't name");
	} finally {
		delete polluted.lint;
	}
	assert.strictEqual(invoke_task.mock.calls.length, 0);
});

test('run_task refuses every invoke of a builtin whose grants name no `invokes`', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: { task: { run: (ctx) => ctx.invoke_task('gen') } }
		},
		{},
		{ ...(await create_mock_run_task_options()), invoke_task, grants: {} }
	);
	assert(!result.ok);
	assert.include(result.reason, "invokes `gen`, which its grants don't name");
	assert.strictEqual(invoke_task.mock.calls.length, 0);
});

test('run_task refuses a task with grants run without its builtin grants, naming the builtin', async () => {
	const run = vi.fn();
	const run_with = async (summary: string) =>
		run_task(
			{
				name: 'src/lib/x.task.ts',
				id: '/p/src/lib/x.task.ts',
				mod: { task: { summary, grants: {}, run } as never }
			},
			{},
			await create_mock_run_task_options()
		);
	const named = await run_with(GRO_BUILTIN_TASK_SUMMARIES.check);
	assert(!named.ok);
	assert.instanceOf(named.error, TaskError);
	assert.include(named.reason, 'declares `grants`, so it runs only as a builtin, by name');
	assert.include(named.reason, "`gro check`, or `invoke_task('check')`");
	const unnamed = await run_with('no builtin has this summary');
	assert(!unnamed.ok);
	assert.include(unnamed.reason, 'runs only as a builtin, by name.');
	assert.strictEqual(run.mock.calls.length, 0);
});
test('run_task refuses a builtin invoking a builtin its `invokes` leaves out', async () => {
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const run_invoking = async (name: string) =>
		run_task(
			{
				name: 'greet',
				id: '/p/src/lib/greet.task.ts',
				mod: { task: { run: (ctx) => ctx.invoke_task(name) } }
			},
			{},
			{ ...(await create_mock_run_task_options()), invoke_task, grants: { invokes: { gen: {} } } }
		);
	assert((await run_invoking('gen')).ok);
	for (const name of ['publish', 'deploy', 'check']) {
		const result = await run_invoking(name);
		assert(!result.ok);
		assert.include(result.reason, `invokes \`${name}\`, which its grants don't name`);
	}
	assert.deepEqual(
		invoke_task.mock.calls.map((call) => call[0]),
		['gen']
	);
});
