import { test, expect } from 'vitest';
import { resolve } from 'node:path';

import { spawn_with_loader } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

test(
	'basic behavior',
	async () => {
		const test_script = resolve('src/test/fixtures/test_run_gen.ts');

		const result = await spawn_with_loader(LOADER_SOURCE_PATH, test_script, []);

		// The spawn_with_loader function handles output automatically
		// Just check if the process succeeded (ok implies code === 0)
		expect(result.ok).toBe(true);
	},
	TEST_TIMEOUT_MD
);
