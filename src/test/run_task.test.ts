import { test, expect, assert, vi } from 'vitest';
import { resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { assert_rejects } from '@fuzdev/fuz_util/testing.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { run_task } from '#lib/run_task.ts';
import { SilentError, TaskError, type Task, type TaskContext } from '#lib/task.ts';
import { task as run_task_task } from '#lib/run.task.ts';

import {
	TEST_TIMEOUT_MD,
	LOADER_SOURCE_PATH,
	create_mock_config,
	create_mock_module_graph,
	create_mock_watcher,
	create_mock_logger,
	create_mock_timings
} from './test_helpers.ts';

test(
	'run_task passes args and returns output',
	async () => {
		const testScript = resolve('src/test/fixtures/test_run_task_basic.ts');

		// Log test script path for debugging
		// eslint-disable-next-line no-console
		console.log('Test script path:', testScript);
		// eslint-disable-next-line no-console
		console.log('Test script exists:', existsSync(testScript));

		const result = await spawn_with_loader(LOADER_SOURCE_PATH, testScript, []);

		expect(result.ok).toBe(true);
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

		const result = await spawn_with_loader(LOADER_SOURCE_PATH, testScript, []);

		expect(result.ok).toBe(true);
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

		const result = await spawn_with_loader(LOADER_SOURCE_PATH, testScript, []);

		expect(result.ok).toBe(true);
	},
	TEST_TIMEOUT_MD
);

test('run_task throws a `TaskError` listing each issue when args fail validation', async () => {
	const run = vi.fn();
	const error = await assert_rejects(async () =>
		run_task(
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
			vi.fn(),
			await create_mock_config(),
			create_mock_module_graph(),
			create_mock_watcher(),
			create_mock_logger(),
			create_mock_timings()
		)
	);
	assert(error instanceof TaskError);
	assert.strictEqual(
		error.message,
		"Failed task args validation for task 'greet':\n" +
			'name: expected string, received number\n' +
			'count: expected number, received string'
	);
	assert.strictEqual(run.mock.calls.length, 0);
});

test('run_task prints the issue of each bad forwarded arg of `gro run`', async () => {
	const error = await assert_rejects(async () =>
		run_task(
			{ name: 'run', id: '/p/src/lib/run.task.ts', mod: { task: run_task_task as Task<any> } },
			{ _: ['script.ts'], x: { a: 1 }, y: null } as any,
			vi.fn(),
			await create_mock_config(),
			create_mock_module_graph(),
			create_mock_watcher(),
			create_mock_logger(),
			create_mock_timings()
		)
	);
	assert(error instanceof TaskError);
	assert.strictEqual(
		error.message,
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
		() => Promise.reject(new SilentError()), // `invoke_task` already logged why
		await create_mock_config(),
		create_mock_module_graph(),
		create_mock_watcher(),
		create_mock_logger(),
		create_mock_timings()
	);
	assert(!result.ok);
	assert(result.error instanceof SilentError);
	assert.include(result.reason, 'failed.');
	assert.notInclude(result.reason, 'Unexpected error');
});

test('run_task passes the graph and watcher to the task and to the tasks it invokes', async () => {
	const graph = create_mock_module_graph();
	const watcher = create_mock_watcher();
	const invoke_task = vi.fn((..._args: Array<unknown>) => Promise.resolve());
	const config = await create_mock_config();
	const result = await run_task(
		{
			name: 'greet',
			id: '/p/src/lib/greet.task.ts',
			mod: {
				task: {
					run: async (ctx) => {
						const task_name: string = 'other'; // widened, not a registered task
						await ctx.invoke_task(task_name);
						return ctx;
					}
				}
			}
		},
		{},
		invoke_task,
		config,
		graph,
		watcher,
		create_mock_logger(),
		create_mock_timings()
	);
	assert(result.ok);
	const ctx = result.output as TaskContext;
	assert.strictEqual(ctx.graph, graph);
	assert.strictEqual(ctx.watcher, watcher);
	expect(invoke_task).toHaveBeenCalledWith(
		'other',
		undefined,
		config,
		{ graph, watcher },
		expect.anything(),
		expect.anything()
	);
});
