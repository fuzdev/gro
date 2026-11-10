import { describe, test, expect, vi, afterEach, onTestFinished } from 'vitest';
import { chmod, mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { spawn_restartable } from '#lib/spawn_restartable.ts';
import { spawn } from '#lib/spawn.ts';

const create_fake = async (name: string, body: string): Promise<{ dir: string; bin: string }> => {
	const dir = await mkdtemp(join(tmpdir(), `gro_fake_${name}_`));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	const bin = join(dir, 'bin');
	await mkdir(bin);
	await writeFile(join(bin, name), `#!/bin/sh\n${body}\n`);
	await chmod(join(bin, name), 0o755);
	return { dir, bin };
};

afterEach(() => {
	vi.unstubAllEnvs();
});

describe('spawn_restartable over the real spawn module', () => {
	test('a restart runs the pinned program though PATH has changed', async () => {
		const first = await create_fake('tool', 'touch "$FAKE_DIR/started-first"\nexit 0');
		const second = await create_fake('tool', 'touch "$FAKE_DIR/started-second"\nexit 0');
		const out = await mkdtemp(join(tmpdir(), 'gro_fake_out_'));
		onTestFinished(() => rm(out, { recursive: true, force: true }));
		vi.stubEnv('PATH', `${first.bin}:${process.env.PATH ?? ''}`);

		const program = spawn.program('tool', { env: { FAKE_DIR: out } });
		const restartable = await spawn_restartable(program);
		await restartable.closed;
		expect(existsSync(join(out, 'started-first'))).toBe(true);

		// a program pinned before this PATH change, so `second` must never run
		vi.stubEnv('PATH', `${second.bin}:${process.env.PATH ?? ''}`);
		await rm(join(out, 'started-first'));
		await restartable.restart();
		await restartable.closed;

		expect(existsSync(join(out, 'started-first'))).toBe(true);
		expect(existsSync(join(out, 'started-second'))).toBe(false);
	});
});
