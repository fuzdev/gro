import { test, expect, assert, vi } from 'vitest';
import { resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { assert_rejects } from '@fuzdev/fuz_util/testing.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { resolve_gro_module_path, spawn_with_loader } from '#lib/gro_helpers.ts';
import { run_task } from '#lib/run_task.ts';
import { TaskError, type Task } from '#lib/task.ts';
import { task as run_task_task } from '#lib/run.task.ts';

import {
	TEST_TIMEOUT_MD,
	create_mock_config,
	create_mock_filer,
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

		// Use the same loader resolution logic as the CLI
		const loaderPath = resolve_gro_module_path('loader.js');

		// Use the existing spawn_with_loader function
		const result = await spawn_with_loader(loaderPath, testScript, []);

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

		// Use the same loader resolution logic as the CLI
		const loaderPath = resolve_gro_module_path('loader.js');

		// Use the existing spawn_with_loader function
		const result = await spawn_with_loader(loaderPath, testScript, []);

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

		// Use the same loader resolution logic as the CLI
		const loaderPath = resolve_gro_module_path('loader.js');

		// Use the existing spawn_with_loader function
		const result = await spawn_with_loader(loaderPath, testScript, []);

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
			create_mock_filer(),
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

test('run_task prints a message set by the schema as it is', async () => {
	const error = await assert_rejects(async () =>
		run_task(
			{ name: 'run', id: '/p/src/lib/run.task.ts', mod: { task: run_task_task as Task<any> } },
			{ _: ['script.ts'], x: { a: 1 } } as any,
			vi.fn(),
			await create_mock_config(),
			create_mock_filer(),
			create_mock_logger(),
			create_mock_timings()
		)
	);
	assert(error instanceof TaskError);
	assert.strictEqual(
		error.message,
		"Failed task args validation for task 'run':\n" +
			'x: expected string | number | boolean | array, received object'
	);
});
