import { test, expect } from 'vitest';
import { resolve } from 'node:path';

import { spawn_with_loader } from '#lib/gro_helpers.ts';

import { TEST_TIMEOUT_MD } from './test_helpers.ts';

test(
	'custom loader works in separate process',
	async () => {
		const test_script = resolve('src/test/fixtures/test_loader.ts');

		// the source loader, not a built `dist/` that may be stale
		const loader_path = resolve('src/lib/loader.ts');

		// Use the existing spawn_with_loader function
		const result = await spawn_with_loader(loader_path, test_script, []);

		// The spawn_with_loader function handles output automatically
		// Just check if the process succeeded (ok implies code === 0)
		expect(result.ok).toBe(true);
	},
	TEST_TIMEOUT_MD
);
