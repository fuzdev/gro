import { test, assert, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

const INVOKE_SOURCE_PATH = resolve('src/lib/invoke.ts');

test(
	'a TaskError raised before any task prints its message with no stack and exits 1',
	async () => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_invoke_errors_'));
		onTestFinished(() => rm(dir, { recursive: true, force: true }));
		// a failing `svelte-kit` CLI, what `sveltekit_sync_if_obviously_needed` runs before any task
		await mkdir(join(dir, 'node_modules/.bin'), { recursive: true });
		const cli = join(dir, 'node_modules/.bin/svelte-kit');
		await writeFile(cli, '#!/bin/sh\nexit 1\n');
		await chmod(cli, 0o755);
		await writeFile(join(dir, 'package.json'), '{"type": "module"}\n');

		const result = spawnSync(
			process.execPath,
			spawn_with_loader_args(LOADER_SOURCE_PATH, INVOKE_SOURCE_PATH, []),
			// the logger goes quiet under `VITEST`, and a child inherits it
			{ cwd: dir, encoding: 'utf8', env: { ...process.env, NO_COLOR: '1', VITEST: '' } }
		);

		assert.equal(result.status, 1);
		assert.include(result.stderr + result.stdout, 'Failed svelte-kit sync');
		assert.notMatch(result.stderr + result.stdout, /^\s+at /m);
	},
	TEST_TIMEOUT_MD
);
