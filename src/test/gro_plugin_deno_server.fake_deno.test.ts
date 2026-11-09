import { describe, test, expect, vi, afterEach, onTestFinished } from 'vitest';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { gro_plugin_deno_server } from '#lib/gro_plugin_deno_server.ts';
import { spawn_restartable } from '#lib/spawn_restartable.ts';
import { spawn } from '#lib/spawn.ts';
import type { PluginContext } from '#lib/plugin.ts';

import { create_mock_logger } from './test_helpers.ts';

/*

A `gro dev`-shaped run: the plugin's setup and teardown over the real spawn module and a fake
`deno`, reporting only whether a variable is set, by name, so no output carries the environment.

*/

const FAKE_DENO_BODY = `
if [ "\${DENO_MARK+x}" = x ] && [ "$PORT" = 4242 ] && [ "$HOST" = 127.0.0.1 ] && [ "$NODE_ENV" = development ]; then
	echo ok > "$DENO_DIR_FAKE/env"
else
	echo bad > "$DENO_DIR_FAKE/env"
fi
printf '%s\\n' "$@" > "$DENO_DIR_FAKE/args"
echo "$$" > "$DENO_DIR_FAKE/pid"
touch "$DENO_DIR_FAKE/ready"
trap 'touch "$DENO_DIR_FAKE/terminated"; exit 0' TERM
while :; do sleep 0.05; done
`;

const create_fake = async (name: string, body: string): Promise<{ dir: string; bin: string }> => {
	const dir = await mkdtemp(join(tmpdir(), `gro_fake_${name}_`));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	const bin = join(dir, 'bin');
	await mkdir(bin);
	await writeFile(join(bin, name), `#!/bin/sh\n${body}\n`);
	await chmod(join(bin, name), 0o755);
	return { dir, bin };
};

const wait_for = async (path: string): Promise<void> => {
	const start = Date.now();
	while (!existsSync(path)) {
		if (Date.now() - start > 5000) throw Error(`timed out waiting for ${path}`);
		await new Promise((r) => setTimeout(r, 20));
	}
};

const is_pid_running = (pid: number): boolean => {
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
};

const to_ctx = (): PluginContext =>
	({ dev: true, watch: false, log: create_mock_logger() }) as unknown as PluginContext;

afterEach(() => {
	vi.unstubAllEnvs();
});

describe('gro_plugin_deno_server over a fake deno', () => {
	test('setup starts a pinned deno with its args and env, teardown ends it', async () => {
		const { dir, bin } = await create_fake('deno', FAKE_DENO_BODY);
		vi.stubEnv('PATH', `${bin}:${process.env.PATH ?? ''}`);
		vi.stubEnv('DENO_MARK', 'x');
		const plugin = gro_plugin_deno_server({
			entry: 'server.ts',
			port: 4242,
			host: '127.0.0.1',
			env_file: null,
			env: { DENO_DIR_FAKE: dir }
		});

		await plugin.setup!(to_ctx());
		await wait_for(join(dir, 'ready'));

		expect((await readFile(join(dir, 'env'), 'utf8')).trim()).toBe('ok');
		expect((await readFile(join(dir, 'args'), 'utf8')).trim().split('\n')).toEqual([
			'run',
			'--allow-net',
			'--allow-read',
			'--allow-env',
			'server.ts'
		]);

		const pid = Number((await readFile(join(dir, 'pid'), 'utf8')).trim());
		// a teardown that failed to stop it must not leave the fake running past the test
		onTestFinished(() => {
			if (is_pid_running(pid)) process.kill(pid, 'SIGKILL');
		});
		expect(is_pid_running(pid)).toBe(true);

		await plugin.teardown!(to_ctx());

		expect(existsSync(join(dir, 'terminated'))).toBe(true);
		expect(is_pid_running(pid)).toBe(false);
	});
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
