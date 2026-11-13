import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { spawn, type SpawnProgram } from '#lib/spawn.ts';
import { is_spawn_error } from '#lib/spawn_error.ts';
import {
	create_fs_sandbox,
	create_removed_dir,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

// grace periods and child startups on a loaded machine, none of them a sleep the assertions rest on
vi.setConfig({ testTimeout: 30_000 });

const SYSTEM_PATH = process.env.PATH;

// past a pipe's buffer, so a write can't complete before someone reads
const LARGE = 2 * 1024 * 1024;

const large_input = (): Uint8Array => Uint8Array.from({ length: LARGE }, (_, i) => i % 251);

let sandbox: FsSandbox;
let bin: string;
let tracked_pids: Array<number>;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	bin = join(sandbox.base, 'bin');
	mkdirSync(bin);
	tracked_pids = [];
	process.env.PATH = [bin, SYSTEM_PATH].join(':');
});

afterEach(() => {
	process.env.PATH = SYSTEM_PATH;
	for (const pid of tracked_pids) {
		try {
			process.kill(pid, 'SIGKILL');
		} catch {
			// already gone
		}
	}
	remove_fs_sandbox(sandbox);
});

const program_of = (name: string, body: string): SpawnProgram => {
	const path = join(bin, name);
	writeFileSync(path, `#!/bin/sh\n${body}\n`);
	chmodSync(path, 0o755);
	return spawn.program(name);
};

const encode = (text: string): Uint8Array => new TextEncoder().encode(text);

const assert_bytes_equal = (actual: Uint8Array, expected: Uint8Array): void => {
	assert.strictEqual(actual.length, expected.length);
	assert.strictEqual(Buffer.compare(actual, expected), 0);
};

const read_pid = async (path: string): Promise<number> => {
	const deadline = Date.now() + 8000;
	while (!existsSync(path) || !readFileSync(path, 'utf8').includes('\n')) {
		if (Date.now() > deadline) assert.fail(`no pid at ${path}`);
		await new Promise((resolve) => setTimeout(resolve, 10));
	}
	const pid = Number(readFileSync(path, 'utf8').trim());
	assert.ok(pid > 1);
	tracked_pids.push(pid);
	return pid;
};

const assert_gone = async (pid: number): Promise<void> => {
	const is_alive = (): boolean => {
		try {
			process.kill(pid, 0);
			return true;
		} catch {
			return false;
		}
	};
	const deadline = Date.now() + 8000;
	while (is_alive()) {
		if (Date.now() > deadline) assert.fail(`pid ${pid} still running`);
		await new Promise((resolve) => setTimeout(resolve, 10));
	}
};

describe('`stdin` data', () => {
	test('`out_bytes` sends every byte value', async () => {
		const input = Uint8Array.from({ length: 256 }, (_, i) => i);
		const output = await spawn.program('cat').out_bytes([], { stdin: input });
		assert.ok(output.kind === 'exited' && output.code === 0);
		assert_bytes_equal(output.stdout, input);
	});

	test('`out` sends data and decodes the output as usual', async () => {
		const output = await spawn.program('cat').out([], { stdin: encode('héllo\n') });
		assert.ok(output.kind === 'exited' && output.code === 0);
		assert.strictEqual(output.stdout, 'héllo\n');
	});

	test('a view sends only its own range', async () => {
		const view = new Uint8Array(encode('abcdef').buffer, 2, 2);
		assert.strictEqual((await spawn.program('cat').out([], { stdin: view })).stdout, 'cd');
		const buffer = Buffer.from('abcdef').subarray(1, 3);
		assert.strictEqual((await spawn.program('cat').out([], { stdin: buffer })).stdout, 'bc');
	});

	test('empty data closes stdin at once', async () => {
		const output = await spawn.program('cat').out([], {
			stdin: new Uint8Array(0),
			timeout_ms: 10_000
		});
		assert.ok(output.kind === 'exited' && output.code === 0, output.kind);
		assert.strictEqual(output.stdout, '');
	});

	test('input and output past the pipe buffer on both streams do not deadlock', async () => {
		const input = large_input();
		// a FIFO, as Node's stdio pipes are sockets and a socket can't be reopened through `/dev/fd`
		const fifo = join(sandbox.base, 'fifo');
		const body = `mkfifo '${fifo}'\ncat '${fifo}' >&2 &\ntee '${fifo}'\nwait`;
		const output = await program_of('both', body).out_bytes([], {
			stdin: input,
			timeout_ms: 20_000
		});
		assert.ok(output.kind === 'exited' && output.code === 0, output.kind);
		assert.strictEqual(output.truncated, false);
		assert_bytes_equal(output.stdout, input);
		assert_bytes_equal(output.stderr, input);
	});

	test('a child that exits without reading is not an error, its result standing', async () => {
		const output = await program_of('leaves', 'exit 3').out_bytes([], { stdin: large_input() });
		assert.ok(output.kind === 'exited' && output.code === 3, output.kind);
		assert.strictEqual(output.truncated, false);
	});

	test('a child that closes stdin and keeps running is not an error', async () => {
		const program = program_of('closes', 'exec 0<&-\nsleep 0.2\necho still here');
		const output = await program.out([], { stdin: large_input() });
		assert.ok(output.kind === 'exited' && output.code === 0, output.kind);
		assert.strictEqual(output.stdout, 'still here\n');
	});

	test('the bytes are copied when the option is read', async () => {
		const input = encode('original');
		const call = spawn.program('cat').out([], { stdin: input });
		input.fill(0x7a);
		assert.strictEqual((await call).stdout, 'original');
	});

	test('the option is read once, and a polluted `Object.prototype.stdin` sets nothing', async () => {
		let reads = 0;
		const options: any = {
			get stdin() {
				reads++;
				return encode(reads === 1 ? 'first' : 'second');
			}
		};
		assert.strictEqual((await spawn.program('cat').out([], options)).stdout, 'first');
		assert.strictEqual(reads, 1);

		(Object.prototype as any).stdin = encode('polluted');
		let polluted;
		try {
			polluted = spawn.program('cat').out([], {});
		} finally {
			delete (Object.prototype as any).stdin;
		}
		assert.strictEqual((await polluted).stdout, '');
	});

	test('anything but a `Uint8Array` or a mode is a TypeError', async () => {
		const ok = program_of('ok', 'exit 0');
		const values: Array<any> = [
			'data',
			new ArrayBuffer(1),
			new DataView(new ArrayBuffer(1)),
			new Uint16Array(1),
			[1],
			{ length: 0 },
			Object.create(Uint8Array.prototype)
		];
		for (const stdin of values) {
			for (const call of [() => ok.out([], { stdin }), () => ok.out_bytes([], { stdin })]) {
				try {
					await call();
					assert.fail('expected a TypeError');
				} catch (error) {
					assert.instanceOf(error, TypeError);
				}
			}
		}
	});

	test('the child leads its own session and group', async () => {
		const own_session = Number(
			readFileSync(`/proc/${process.pid}/stat`, 'utf8').split(') ')[1]!.split(' ')[3]
		);
		const output = await spawn
			.program('sh')
			.out(['-c', 'cat /proc/$$/stat'], { stdin: encode('unread') });
		const stat = output.stdout;
		const pid = Number(stat.slice(0, stat.indexOf(' ')));
		const rest = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
		assert.notStrictEqual(Number(rest[3]), own_session);
		assert.strictEqual(Number(rest[3]), pid);
		assert.strictEqual(Number(rest[2]), pid);
	});

	test('a launch that fails rejects as usual', async () => {
		try {
			await spawn
				.program('cat')
				.out([], { stdin: large_input(), cwd: create_removed_dir(sandbox) });
			assert.fail('expected launch_failed');
		} catch (error) {
			assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
		}
	});
});

describe('`stdin` data, ended mid-write', () => {
	// reads nothing, so the write can't finish
	const stuck = (): SpawnProgram => program_of('stuck', 'exec sleep 30');

	test('`timeout_ms` ends the call', async () => {
		const output = await stuck().out_bytes([], { stdin: large_input(), timeout_ms: 100 });
		assert.strictEqual(output.kind, 'timed_out');
	});

	test('`signal` ends the call', async () => {
		const controller = new AbortController();
		setTimeout(() => controller.abort(), 100);
		const output = await stuck().out([], { stdin: large_input(), signal: controller.signal });
		assert.strictEqual(output.kind, 'aborted');
	});

	test('the cap ends the call', async () => {
		const output = await program_of('over', 'head -c 200000 /dev/zero\nexec sleep 30').out_bytes(
			[],
			{ stdin: large_input(), max_bytes: 1024 }
		);
		assert.strictEqual(output.truncated, true);
		assert.ok(output.kind === 'signaled' && output.signal === 'SIGKILL', output.kind);
	});

	test('a stop reaches what the child left in its group', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const program = program_of('leaves_one', `sleep 30 &\necho $! > '${pid_path}'\nwait`);
		const output = await program.out([], { stdin: large_input(), timeout_ms: 300 });
		assert.strictEqual(output.kind, 'timed_out');
		await assert_gone(await read_pid(pid_path));
	});
});
