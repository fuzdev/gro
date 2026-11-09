import { describe, test, assert, vi } from 'vitest';
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { spawn, type SpawnProgram } from '#lib/spawn.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const SYSTEM_PATH = process.env.PATH;

const with_program = async (
	name: string,
	body: (sandbox: FsSandbox) => string,
	fn: (program: SpawnProgram, sandbox: FsSandbox) => Promise<void>
): Promise<void> => {
	const sandbox = await create_fs_sandbox();
	const bin = join(sandbox.base, 'bin');
	mkdirSync(bin);
	process.env.PATH = [bin, SYSTEM_PATH].join(':');
	try {
		const path = join(bin, name);
		writeFileSync(path, `#!/bin/sh\n${body(sandbox)}\n`);
		chmodSync(path, 0o755);
		await fn(spawn.program(name), sandbox);
	} finally {
		process.env.PATH = SYSTEM_PATH;
		remove_fs_sandbox(sandbox);
	}
};

describe('the capture cap', () => {
	test('with stdin inherited, the cap ends the call past a process still holding a pipe', async () => {
		const pid_name = 'background_pid';
		await with_program(
			'held',
			(sandbox) =>
				`sleep 100 &\necho $! > '${join(sandbox.base, pid_name)}'\nhead -c 200000 /dev/zero\nwait`,
			async (program, sandbox) => {
				const started = performance.now();
				const output = await program.out_bytes([], { max_bytes: 1024, stdin: 'inherit' });
				const elapsed = performance.now() - started;
				const pid = Number(readFileSync(join(sandbox.base, pid_name), 'utf8').trim());
				try {
					assert.strictEqual(output.truncated, true);
					assert.ok(output.kind === 'signaled' && output.signal === 'SIGKILL');
					// well under the wait for a held pipe, which dropping the pipes skips
					assert.isBelow(elapsed, 1500);
				} finally {
					process.kill(pid, 'SIGKILL');
				}
			}
		);
	});

	test('`out_bytes` defaults to 64 MiB', async () => {
		const cap = 64 * 1024 * 1024;
		await with_program(
			'over_default',
			() => `head -c ${cap + 1} /dev/zero; exec sleep 30`,
			async (program) => {
				const output = await program.out_bytes([]);
				assert.strictEqual(output.truncated, true);
				assert.strictEqual(output.stdout.length, cap);
				assert.ok(output.kind === 'signaled' && output.signal === 'SIGKILL');
			}
		);
	});
});
