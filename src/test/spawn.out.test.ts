import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import {
	chmodSync,
	existsSync,
	mkdirSync,
	readFileSync,
	readlinkSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';

import { spawn, type SpawnOutOptions, type SpawnProgram } from '#lib/spawn.ts';
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

const STDINS = ['ignore', 'inherit'] as const;

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

interface ProcStat {
	pid: number;
	pgrp: number;
	session: number;
}

// past the last `)`, since the command name in parens can hold spaces
const parse_stat = (stat: string): ProcStat => {
	const rest = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
	return {
		pid: Number(stat.slice(0, stat.indexOf(' '))),
		pgrp: Number(rest[2]),
		session: Number(rest[3])
	};
};

const host_stat = (): ProcStat => parse_stat(readFileSync(`/proc/${process.pid}/stat`, 'utf8'));

const to_bytes = (...values: Array<number>): Uint8Array => new Uint8Array(values);

const ALL_BYTES = Uint8Array.from({ length: 256 }, (_, i) => i);

const assert_plain_bytes = (actual: Uint8Array, expected: Uint8Array): void => {
	assert.strictEqual(Object.getPrototypeOf(actual), Uint8Array.prototype);
	assert.strictEqual(actual.byteOffset, 0);
	assert.strictEqual(actual.buffer.byteLength, expected.length);
	assert.deepEqual([...actual], [...expected]);
};

const poll_alive = (pid: number): boolean => {
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
};

const assert_gone = async (pid: number): Promise<void> => {
	const deadline = Date.now() + 8000;
	while (poll_alive(pid)) {
		if (Date.now() > deadline) assert.fail(`pid ${pid} still running`);
		await new Promise((resolve) => setTimeout(resolve, 10));
	}
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

describe('`stdin` and the session', () => {
	const stat_script = 'cat /proc/$$/stat';

	for (const method of ['out', 'out_bytes'] as const) {
		const read_stat = async (options?: SpawnOutOptions): Promise<ProcStat> => {
			const sh = spawn.program('sh');
			if (method === 'out') {
				const output = await sh.out(['-c', stat_script], options);
				assert.ok(output.kind === 'exited' && output.code === 0);
				return parse_stat(output.stdout);
			}
			const output = await sh.out_bytes(['-c', stat_script], options);
			assert.ok(output.kind === 'exited' && output.code === 0);
			return parse_stat(new TextDecoder().decode(output.stdout));
		};

		test(`${method}: \`stdin: 'inherit'\` keeps the child in the host's session and group`, async () => {
			const own = host_stat();
			const child = await read_stat({ stdin: 'inherit' });
			assert.strictEqual(child.session, own.session);
			assert.strictEqual(child.pgrp, own.pgrp);
		});

		test(`${method}: by default the child leads a new session`, async () => {
			const own = host_stat();
			for (const options of [undefined, {}, { stdin: 'ignore' } as const]) {
				const child = await read_stat(options);
				assert.notStrictEqual(child.session, own.session);
				assert.strictEqual(child.session, child.pid);
				assert.strictEqual(child.pgrp, child.pid);
			}
		});
	}

	test("stdin is the host's with `inherit` and `/dev/null` otherwise", async () => {
		const readlink = spawn.program('readlink');
		const own = readlinkSync('/proc/self/fd/0');
		const inherited = await readlink.out(['/proc/self/fd/0'], { stdin: 'inherit' });
		assert.strictEqual(inherited.stdout, own + '\n');
		const ignored = await readlink.out(['/proc/self/fd/0'], { stdin: 'ignore' });
		assert.strictEqual(ignored.stdout, '/dev/null\n');
	});

	test('a narrowed handle captures the same', async () => {
		const sh = spawn.narrow(['sh']).program('sh');
		const own = host_stat();
		const inherited = await sh.out(['-c', stat_script], { stdin: 'inherit' });
		assert.strictEqual(parse_stat(inherited.stdout).session, own.session);
		const bytes = await sh.out_bytes(['-c', "printf '\\377'"]);
		assert_plain_bytes(bytes.stdout, to_bytes(0xff));
	});

	test('a bad `stdin` is a TypeError', async () => {
		const ok = program_of('ok', 'exit 0');
		for (const stdin of ['pipe', 'nope', 1, null, ['inherit']] as Array<any>) {
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

	test('a polluted `Object.prototype.stdin` sets nothing, and the option is read once', async () => {
		const sh = spawn.program('sh');
		const own = host_stat();
		(Object.prototype as any).stdin = 'inherit';
		let polluted;
		try {
			polluted = sh.out(['-c', stat_script], {});
		} finally {
			delete (Object.prototype as any).stdin;
		}
		assert.notStrictEqual(parse_stat((await polluted).stdout).session, own.session);

		let reads = 0;
		const options: any = {
			get stdin() {
				reads++;
				return reads === 1 ? 'inherit' : 'ignore';
			}
		};
		const output = await sh.out(['-c', stat_script], options);
		assert.strictEqual(reads, 1);
		assert.strictEqual(parse_stat(output.stdout).session, own.session);
	});
});

describe('byte capture', () => {
	for (const stdin of STDINS) {
		test(`every byte value round-trips on both streams, stdin ${stdin}`, async () => {
			const path = join(sandbox.base, 'all_bytes');
			writeFileSync(path, ALL_BYTES);
			const sh = spawn.program('sh');
			const output = await sh.out_bytes(['-c', 'cat "$1"; cat "$1" >&2', 'sh', path], { stdin });
			assert.ok(output.kind === 'exited' && output.code === 0);
			assert_plain_bytes(output.stdout, ALL_BYTES);
			assert_plain_bytes(output.stderr, ALL_BYTES);
			assert.strictEqual(output.truncated, false);
		});
	}

	test('invalid UTF-8 is exact as bytes and `U+FFFD` as text', async () => {
		const sh = spawn.program('sh');
		const args = ['-c', "printf 'a\\377\\376b'"];
		assert_plain_bytes((await sh.out_bytes(args)).stdout, to_bytes(0x61, 0xff, 0xfe, 0x62));
		assert.strictEqual((await sh.out(args)).stdout, 'a��b');
	});

	test('a BOM is kept in both modes', async () => {
		const sh = spawn.program('sh');
		const args = ['-c', "printf '\\357\\273\\277x'"];
		assert_plain_bytes((await sh.out_bytes(args)).stdout, to_bytes(0xef, 0xbb, 0xbf, 0x78));
		assert.strictEqual((await sh.out(args)).stdout, '﻿x');
	});

	test('empty output is an empty array', async () => {
		const output = await program_of('quiet', 'exit 0').out_bytes([]);
		assert_plain_bytes(output.stdout, to_bytes());
		assert_plain_bytes(output.stderr, to_bytes());
	});

	test('a program that can not start throws, and a pre-aborted signal starts nothing', async () => {
		const ok = program_of('ok', 'exit 0');
		const missing = create_removed_dir(sandbox);
		for (const options of [{ cwd: missing }, { signal: AbortSignal.abort() }]) {
			try {
				await ok.out_bytes([], options);
				assert.fail('expected a launch failure');
			} catch (error) {
				assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
			}
		}
	});
});

describe('text decoding', () => {
	test('a multi-byte character split across chunks stays one character', async () => {
		const sh = spawn.program('sh');
		const args = ['-c', "printf '\\342'; sleep 0.2; printf '\\202\\254'"];
		assert.strictEqual((await sh.out(args)).stdout, '€');
		assert.strictEqual((await sh.out(args, { stdin: 'inherit' })).stdout, '€');
		assert_plain_bytes((await sh.out_bytes(args)).stdout, to_bytes(0xe2, 0x82, 0xac));
	});
});

describe('the cap', () => {
	// `head` bounds the output, and the exec'd `sleep` is what only the cap ends
	const body = 'head -c 200000 /dev/zero; exec sleep 30';

	for (const stdin of STDINS) {
		test(`\`out\` keeps \`max_bytes\` and reports \`truncated\`, stdin ${stdin}`, async () => {
			const output = await program_of('over', body).out([], { max_bytes: 1024, stdin });
			assert.strictEqual(output.truncated, true);
			assert.strictEqual(output.stdout.length, 1024);
			assert.ok(output.kind === 'signaled' && output.signal === 'SIGKILL');
		});

		test(`\`out_bytes\` keeps \`max_bytes\` and reports \`truncated\`, stdin ${stdin}`, async () => {
			const output = await program_of('over', body).out_bytes([], { max_bytes: 1024, stdin });
			assert.strictEqual(output.truncated, true);
			assert_plain_bytes(output.stdout, new Uint8Array(1024));
			assert.ok(output.kind === 'signaled' && output.signal === 'SIGKILL');
		});
	}

	test('the cap is per stream, stderr included', async () => {
		const program = program_of('over_err', 'head -c 200000 /dev/zero >&2; exec sleep 30');
		const output = await program.out_bytes([], { max_bytes: 1024 });
		assert.strictEqual(output.truncated, true);
		assert_plain_bytes(output.stderr, new Uint8Array(1024));
		assert.strictEqual(output.stdout.length, 0);
	});

	test('exactly at the cap is not truncated, and a cap of 0 keeps nothing', async () => {
		const program = program_of('five', 'printf 12345');
		const exact = await program.out_bytes([], { max_bytes: 5 });
		assert.strictEqual(exact.truncated, false);
		assert_plain_bytes(exact.stdout, to_bytes(0x31, 0x32, 0x33, 0x34, 0x35));
		const none = await program.out_bytes([], { max_bytes: 0 });
		assert.strictEqual(none.truncated, true);
		assert_plain_bytes(none.stdout, to_bytes());
	});

	test('a bad `max_bytes` is a TypeError on `out_bytes`', async () => {
		const ok = program_of('ok', 'exit 0');
		for (const max_bytes of [-1, 1.5, NaN, '1' as any, null as any]) {
			try {
				await ok.out_bytes([], { max_bytes });
				assert.fail('expected a TypeError');
			} catch (error) {
				assert.instanceOf(error, TypeError);
			}
		}
	});
});

describe('a background process holding a pipe', () => {
	const hold_script = (pid_path: string): string =>
		`sleep 100 &\necho $! > '${pid_path}'\nprintf text\nexit 2`;

	test('with stdin inherited, `out` reports `truncated`, and the process outlives the call', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const output = await program_of('hold', hold_script(pid_path)).out([], { stdin: 'inherit' });
		assert.ok(output.kind === 'exited' && output.code === 2);
		assert.strictEqual(output.stdout, 'text');
		assert.strictEqual(output.truncated, true);
		assert.ok(poll_alive(await read_pid(pid_path)), 'pid alone leaves the background process');
	});

	test('with stdin inherited, `out_bytes` reports `truncated`', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const output = await program_of('hold', hold_script(pid_path)).out_bytes([], {
			stdin: 'inherit'
		});
		assert.strictEqual(output.truncated, true);
		assert_plain_bytes(output.stdout, to_bytes(0x74, 0x65, 0x78, 0x74));
		await read_pid(pid_path);
	});

	test('by default, `out_bytes` reports `truncated` and the group is killed', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const output = await program_of('hold', hold_script(pid_path)).out_bytes([]);
		assert.strictEqual(output.truncated, true);
		await assert_gone(await read_pid(pid_path));
	});

	test('with stdin inherited, a process that ends inside the bound is not truncated', async () => {
		const output = await program_of(
			'quickbg',
			`(while kill -0 $$ 2>/dev/null; do sleep 0.01; done; printf late) &\nprintf early`
		).out([], { stdin: 'inherit' });
		assert.strictEqual(output.stdout, 'earlylate');
		assert.strictEqual(output.truncated, false);
	});
});

describe('timeouts and aborts', () => {
	for (const stdin of STDINS) {
		for (const method of ['out', 'out_bytes'] as const) {
			test(`${method} with stdin ${stdin}: \`timeout_ms\` is \`timed_out\``, async () => {
				const output = await spawn.program('sleep')[method](['30'], { stdin, timeout_ms: 50 });
				assert.strictEqual(output.kind, 'timed_out');
				assert.ok(output.kind === 'timed_out' && output.signal === 'SIGTERM');
				assert.isNull(output.kind === 'timed_out' ? output.code : 0);
				assert.strictEqual(output.truncated, false);
			});

			test(`${method} with stdin ${stdin}: an abort is \`aborted\``, async () => {
				const controller = new AbortController();
				const call = spawn.program('sleep')[method](['30'], {
					stdin,
					signal: controller.signal
				});
				setTimeout(() => controller.abort(), 50);
				const output = await call;
				assert.strictEqual(output.kind, 'aborted');
				assert.ok(output.kind === 'aborted' && output.signal === 'SIGTERM');
			});
		}
	}

	const stop_script = (pid_path: string): string => `sleep 100 &\necho $! > '${pid_path}'\nwait`;

	test('a stop with stdin inherited reaches the child by pid alone', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const output = await program_of('stopped', stop_script(pid_path)).out([], {
			stdin: 'inherit',
			timeout_ms: 300
		});
		assert.strictEqual(output.kind, 'timed_out');
		assert.strictEqual(output.truncated, true);
		assert.ok(poll_alive(await read_pid(pid_path)), 'the grandchild outlives a pid-only stop');
	});

	test('a stop by default reaches the group', async () => {
		const pid_path = join(sandbox.base, 'grandchild_pid');
		const output = await program_of('stopped', stop_script(pid_path)).out([], { timeout_ms: 300 });
		assert.strictEqual(output.kind, 'timed_out');
		await assert_gone(await read_pid(pid_path));
	});
});

describe('both streams past the pipe buffer', () => {
	const SIZE = 300_000;
	const body = `head -c ${SIZE} /dev/zero | tr '\\0' a\nhead -c ${SIZE} /dev/zero | tr '\\0' b >&2`;

	for (const stdin of STDINS) {
		test(`\`out\` drains both at once, stdin ${stdin}`, async () => {
			const output = await program_of('both', body).out([], { stdin, timeout_ms: 15_000 });
			assert.ok(output.kind === 'exited' && output.code === 0, JSON.stringify(output.kind));
			assert.strictEqual(output.stdout, 'a'.repeat(SIZE));
			assert.strictEqual(output.stderr, 'b'.repeat(SIZE));
			assert.strictEqual(output.truncated, false);
		});

		test(`\`out_bytes\` drains both at once, stdin ${stdin}`, async () => {
			const output = await program_of('both', body).out_bytes([], { stdin, timeout_ms: 15_000 });
			assert.ok(output.kind === 'exited' && output.code === 0, JSON.stringify(output.kind));
			assert_plain_bytes(output.stdout, new Uint8Array(SIZE).fill(0x61));
			assert_plain_bytes(output.stderr, new Uint8Array(SIZE).fill(0x62));
			assert.strictEqual(output.truncated, false);
		});
	}
});
