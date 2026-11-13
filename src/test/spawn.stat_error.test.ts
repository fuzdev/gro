import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { spawn } from '#lib/spawn.ts';
import { is_spawn_error } from '#lib/spawn_error.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

// a stat that fails at a path the walk or the `PATH` entry already reached, which the kernel
// gives only to a path changed after the lookup or a failing filesystem, so it's injected
const stat_failures = vi.hoisted(() => new Map<string, string>());

vi.mock('node:fs', async (import_original) => {
	const original = await import_original<typeof import('node:fs')>();
	const statSync = ((path: unknown, options?: unknown) => {
		const code = typeof path === 'string' ? stat_failures.get(path) : undefined;
		if (code !== undefined) {
			throw Object.assign(new Error(`${code}: injected, stat '${path}'`), { code });
		}
		return (original.statSync as (path: unknown, options?: unknown) => unknown)(path, options);
	}) as typeof original.statSync;
	return { ...original, statSync };
});

const SYSTEM_PATH = process.env.PATH;

let sandbox: FsSandbox;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(() => {
	stat_failures.clear();
	process.env.PATH = SYSTEM_PATH;
	remove_fs_sandbox(sandbox);
});

const write_script = (path: string): void => {
	mkdirSync(join(path, '..'), { recursive: true });
	writeFileSync(path, '#!/bin/sh\nexit 0\n');
	chmodSync(path, 0o755);
};

describe('a stat that fails after the lookup', () => {
	test('beneath a handle is `not_executable` with the stat error, never a miss', () => {
		const path = join(sandbox.root, 'tool');
		write_script(path);
		stat_failures.set(path, 'EACCES');
		for (const find of [
			() => spawn.find_in(sandbox.dir, 'tool'),
			() => spawn.program_in(sandbox.dir, 'tool')
		]) {
			try {
				find();
				assert.fail('expected not_executable');
			} catch (error) {
				assert.ok(is_spawn_error(error, 'not_executable'), String(error));
				assert.include(error.message, path);
				assert.strictEqual((error.cause as { code?: unknown }).code, 'EACCES');
			}
		}
	});

	test('on `PATH` reads as missing, so the walk goes on to the next entry', () => {
		const first = join(sandbox.root, 'first');
		const second = join(sandbox.root, 'second');
		write_script(join(first, 'tool'));
		write_script(join(second, 'tool'));
		stat_failures.set(join(first, 'tool'), 'EACCES');
		process.env.PATH = [first, second].join(':');
		assert.strictEqual(spawn.find('tool')?.path, join(second, 'tool'));
	});

	test('beneath the project reads as missing, so the name is `not_found`', () => {
		// a name with `/` resolves beneath the working directory, the checkout
		const name = 'node_modules/.bin/tsc';
		stat_failures.set(join(process.cwd(), name), 'EACCES');
		assert.isNull(spawn.find(name));
	});
});
