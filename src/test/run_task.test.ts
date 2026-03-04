import {test, expect} from 'vitest';
import {resolve} from 'node:path';
import {existsSync} from 'node:fs';

import {resolve_gro_module_path, spawn_with_loader} from '../lib/gro_helpers.ts';

import {TEST_TIMEOUT_MD} from './test_helpers.ts';

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
	TEST_TIMEOUT_MD,
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
	TEST_TIMEOUT_MD,
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
	TEST_TIMEOUT_MD,
);
