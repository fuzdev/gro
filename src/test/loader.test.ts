import { test, expect } from 'vitest';
import { resolve } from 'node:path';

import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { spawn } from '#lib/spawn.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

const node = spawn.program('node', { pass_env: 'all' });

test(
	'custom loader works in separate process',
	async () => {
		const test_script = resolve('src/test/fixtures/test_loader.ts');

		const result = await spawn_with_loader(node, LOADER_SOURCE_PATH, test_script, [], undefined);

		// The spawn_with_loader function handles output automatically
		expect(result).toEqual({ kind: 'exited', code: 0 });
	},
	TEST_TIMEOUT_MD
);
