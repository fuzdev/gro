import { test, assert, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

const INVOKE_SOURCE_PATH = resolve('src/lib/invoke.ts');

/**
 * Runs the CLI from source with `argv` in a temp project holding `file` with `contents`,
 * whose local Gro (`node_modules/.bin/gro`) has a `loader.js` sibling
 * re-exporting the source loader, which `gro run` resolves and spawns the script with.
 */
const run_cli = async (
	argv: Array<string>,
	file: string,
	contents: string
): Promise<{ status: number | null; output: string }> => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_run_exit_code_'));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	await mkdir(join(dir, 'node_modules/.bin'), { recursive: true });
	await mkdir(join(dir, 'local_gro'));
	await writeFile(join(dir, 'local_gro/gro'), '');
	await writeFile(
		join(dir, 'local_gro/loader.js'),
		`export * from ${JSON.stringify(LOADER_SOURCE_PATH)};\n`
	);
	await symlink(join(dir, 'local_gro/gro'), join(dir, 'node_modules/.bin/gro'));
	await writeFile(join(dir, 'package.json'), '{"type": "module"}\n');
	await writeFile(join(dir, file), contents);
	const result = spawnSync(
		process.execPath,
		spawn_with_loader_args(LOADER_SOURCE_PATH, INVOKE_SOURCE_PATH, argv),
		{ cwd: dir, encoding: 'utf8', env: { ...process.env, NO_COLOR: '1' } }
	);
	return { status: result.status, output: result.stdout + result.stderr };
};

const gro_run = (script: string) => run_cli(['run', 'script.ts'], 'script.ts', script);

test(
	"`gro run` exits with the script's exit code, or a shell's for a signal, and prints no timings",
	async () => {
		const failed = await gro_run("console.log('ran');\nprocess.exit(3);\n");
		assert.equal(failed.status, 3);
		assert.include(failed.output, 'ran');
		assert.notInclude(failed.output, '🕒');

		// a signal exits like a shell, `128` plus its number
		const signaled = await gro_run("process.kill(process.pid, 'SIGTERM');\n");
		assert.equal(signaled.status, 143);
		assert.notInclude(signaled.output, '🕒');

		const succeeded = await gro_run("console.log('ran');\n");
		assert.equal(succeeded.status, 0);
		assert.include(succeeded.output, 'ran');
		assert.notInclude(succeeded.output, '🕒');
	},
	TEST_TIMEOUT_MD
);

test(
	"the CLI exits with a failed task's `exit_code` only when it's 1-255",
	async () => {
		const run_task_exiting = (exit_code: number) =>
			run_cli(
				['task', 'x.task.ts'],
				'x.task.ts',
				`export const task = {run: () => { throw Object.assign(new Error('failed'), {exit_code: ${exit_code}}); }};\n`
			);
		assert.equal((await run_task_exiting(7)).status, 7);
		assert.equal((await run_task_exiting(255)).status, 255);
		// a process exit code is a byte, so `256` would exit `0`
		assert.equal((await run_task_exiting(256)).status, 1);
		assert.equal((await run_task_exiting(0)).status, 1);
	},
	TEST_TIMEOUT_MD
);
