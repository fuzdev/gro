import {test, expect} from 'vitest';
import {resolve} from 'node:path';

import {resolve_gro_module_path, spawn_with_loader} from '../lib/gro_helpers.js';

import {TEST_TIMEOUT_MD} from './test_helpers.ts';

test(
	'basic behavior',
	async () => {
		const test_script = resolve('src/test/fixtures/test_run_gen.ts');

		// Use the same loader resolution logic as the CLI
		const loader_path = resolve_gro_module_path('loader.js');

		// Use the existing spawn_with_loader function
		const result = await spawn_with_loader(loader_path, test_script, []);

		// The spawn_with_loader function handles output automatically
		// Just check if the process succeeded (ok implies code === 0)
		expect(result.ok).toBe(true);
	},
	TEST_TIMEOUT_MD,
);
