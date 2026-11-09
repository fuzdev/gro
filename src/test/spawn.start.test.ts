import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import { getEventListeners } from 'node:events';
import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';
import { spawn, type Spawn, type SpawnChild, type SpawnProgram } from '#lib/spawn.ts';
import { is_spawn_error } from '#lib/spawn_error.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

// grace periods and child startups on a loaded machine, none of them a sleep the assertions rest on
vi.setConfig({ testTimeout: 30_000 });

const SYSTEM_PATH = process.env.PATH;
// before any group runs, as a stop's grace kill can keep a group tracked past its test
const SIGTERM_LISTENERS = process.listenerCount('SIGTERM');

let sandbox: FsSandbox;
let bin: string;
// everything a test made a child or grandchild report, killed after the test whatever happened
let tracked_pids: Array<number>;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	bin = join(sandbox.base, 'bin');
	mkdirSync(bin);
	tracked_pids = [];
	process.env.PATH = [bin, SYSTEM_PATH].join(':');
});

afterEach(async () => {
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

const write_script = (path: string, body: string, mode = 0o755): void => {
	writeFileSync(path, `#!/bin/sh\n${body}\n`);
	chmodSync(path, mode);
};

const program_of = (name: string, body: string): SpawnProgram => {
	write_script(join(bin, name), body);
	return spawn.program(name);
};

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

const poll_until = async (check: () => boolean, label: string, ms = 8000): Promise<void> => {
	const deadline = Date.now() + ms;
	while (!check()) {
		if (Date.now() > deadline) assert.fail(`timed out waiting for ${label}`);
		await sleep(10);
	}
};

const is_process_alive = (pid: number): boolean => {
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
};

const read_file_when_ready = async (path: string): Promise<string> => {
	await poll_until(() => existsSync(path) && readFileSync(path, 'utf8').includes('\n'), path);
	return readFileSync(path, 'utf8');
};

const read_pid_when_ready = async (path: string): Promise<number> => {
	const pid = Number((await read_file_when_ready(path)).trim());
	assert.ok(pid > 1, `read pid ${pid}`);
	tracked_pids.push(pid);
	return pid;
};

const assert_gone = (pid: number, label: string): Promise<void> =>
	poll_until(() => !is_process_alive(pid), `${label} (pid ${pid}) to end`);

const read_all = async (stream: AsyncIterable<Uint8Array>): Promise<string> => {
	let text = '';
	const decoder = new TextDecoder();
	for await (const chunk of stream) text += decoder.decode(chunk, { stream: true });
	return text + decoder.decode();
};

describe('start', () => {
	test('pipes stdin, stdout, and stderr, with plain `Uint8Array` chunks', async () => {
		const program = program_of('echoes', 'echo out; echo err >&2; cat');
		const child = await program.start([], { stdio: 'pipe' });
		assert.ok(child.pid > 1);
		const stdin_write = await child.stdin!.write('from stdin\n');
		assert.strictEqual(stdin_write, true);
		await child.stdin!.write(new TextEncoder().encode('more\n'));
		await child.stdin!.close();
		const chunks: Array<Uint8Array> = [];
		for await (const chunk of child.stdout!) chunks.push(chunk);
		assert.ok(chunks.length > 0);
		for (const chunk of chunks) {
			assert.strictEqual(Object.getPrototypeOf(chunk), Uint8Array.prototype);
			assert.isFalse(Buffer.isBuffer(chunk));
		}
		const stdout = Buffer.concat(chunks).toString();
		assert.strictEqual(stdout, 'out\nfrom stdin\nmore\n');
		assert.strictEqual(await read_all(child.stderr!), 'err\n');
		assert.deepEqual({ ...(await child.closed) }, { kind: 'exited', code: 0 });
	});

	test('per-stream stdio gives a null for a stream that is not piped', async () => {
		const program = program_of('quiet', 'exit 0');
		const child = await program.start([], { stdio: ['ignore', 'pipe', 'ignore'] });
		assert.isNull(child.stdin);
		assert.isNull(child.stderr);
		assert.ok(child.stdout);
		assert.deepEqual({ ...(await child.closed) }, { kind: 'exited', code: 0 });
		const inherited = await program.start([]);
		assert.isNull(inherited.stdin);
		assert.isNull(inherited.stdout);
		assert.isNull(inherited.stderr);
		await inherited.closed;
	});

	test('a piped stream keeps its output until a reader arrives', async () => {
		const child = await program_of('early', 'echo early').start([], {
			stdio: ['ignore', 'pipe', 'ignore']
		});
		await child.closed;
		assert.strictEqual(await read_all(child.stdout!), 'early\n');
	});

	test('more output than the buffer reaches a late reader in full, and `closed` is not held by it', async () => {
		// 3 MiB, past the buffer, so the stream pauses and resumes for the reader
		const child = await program_of('lots', 'dd if=/dev/zero bs=65536 count=48 2>/dev/null').start(
			[],
			{ stdio: ['ignore', 'pipe', 'ignore'] }
		);
		let total = 0;
		for await (const chunk of child.stdout!) total += chunk.length;
		assert.strictEqual(total, 3 * 1024 * 1024);
		assert.deepEqual({ ...(await child.closed) }, { kind: 'exited', code: 0 });
	});

	test('a reader behind at the drain bound still gets all the output of an exited child', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		// past the buffer but within what the pipe holds, so the child exits with the stream paused
		const child = await program_of(
			'behind',
			`sleep 100 &\necho $! > '${grandchild_path}'\ndd if=/dev/zero bs=65536 count=17 2>/dev/null`
		).start([], { stdio: ['ignore', 'pipe', 'ignore'] });
		const grandchild = await read_pid_when_ready(grandchild_path);
		// claimed with nothing read, so the whole output counts against the buffer
		const iterator = child.stdout![Symbol.asyncIterator]();
		await child.closed;
		await assert_gone(grandchild, 'the grandchild');
		let total = 0;
		for (let next = await iterator.next(); !next.done; next = await iterator.next()) {
			total += next.value.length;
		}
		assert.strictEqual(total, 17 * 65536);
	});

	test('a stream nobody reads pauses, so the child blocks until a reader arrives', async () => {
		// far past the buffer and the pipe, so with the stream paused this can't exit
		const child = await program_of(
			'floods',
			'dd if=/dev/zero bs=65536 count=128 2>/dev/null'
		).start([], { stdio: ['ignore', 'pipe', 'ignore'] });
		const outcome = await Promise.race([
			child.closed.then(() => 'closed'),
			sleep(500).then(() => 'blocked')
		]);
		assert.strictEqual(outcome, 'blocked');
		let total = 0;
		for await (const chunk of child.stdout!) total += chunk.length;
		assert.strictEqual(total, 128 * 65536);
		assert.deepEqual({ ...(await child.closed) }, { kind: 'exited', code: 0 });
	});

	test('a second iteration is refused, and leaving early discards the rest', async () => {
		const ready = join(sandbox.base, 'ready');
		const child = await program_of(
			'noisy',
			`echo first; read _; touch '${ready}'; dd if=/dev/zero bs=65536 count=64 2>/dev/null || exit 8; exit 7`
		).start([], { stdio: ['pipe', 'pipe', 'ignore'] });
		const stdout = child.stdout!;
		const iterator = stdout[Symbol.asyncIterator]();
		assert.throws(() => stdout[Symbol.asyncIterator](), TypeError);
		const first = await iterator.next();
		assert.ok(!first.done);
		assert.strictEqual(new TextDecoder().decode(first.value), 'first\n');
		await iterator.return!();
		assert.isTrue((await iterator.next()).done);
		// with the reader gone, the child still writes all of its output and exits
		await child.stdin!.write('go\n');
		await poll_until(() => existsSync(ready), 'the child to continue');
		assert.deepEqual({ ...(await child.closed) }, { kind: 'exited', code: 7 });
	});

	test('`next` results and the iterator are null-prototype and frozen, ending done', async () => {
		const child = await program_of('hi', 'printf hi').start([], {
			stdio: ['ignore', 'pipe', 'ignore']
		});
		const iterator = child.stdout![Symbol.asyncIterator]();
		assert.ok(Object.isFrozen(iterator));
		const result = await iterator.next();
		assert.isNull(Object.getPrototypeOf(result));
		assert.isFalse(result.done);
		const end = await iterator.next();
		assert.isTrue(end.done);
		assert.isNull(Object.getPrototypeOf(end));
		await child.closed;
	});

	test('concurrent `next` calls each get a chunk or the end', async () => {
		// each `read` holds the next write until the chunk before it was seen, so none can coalesce
		const child = await program_of('two', 'printf a; read _; printf b; read _').start([], {
			stdio: ['pipe', 'pipe', 'ignore']
		});
		const iterator = child.stdout![Symbol.asyncIterator]();
		const pending = [iterator.next(), iterator.next(), iterator.next()];
		const decoder = new TextDecoder();
		const a = await pending[0]!;
		assert.strictEqual(a.done ? null : decoder.decode(a.value), 'a');
		await child.stdin!.write('go\n');
		const b = await pending[1]!;
		assert.strictEqual(b.done ? null : decoder.decode(b.value), 'b');
		await child.stdin!.close();
		assert.isTrue((await pending[2]!).done);
		await child.closed;
	});

	test('a launch failure throws `launch_failed`, `closed` never rejects', async () => {
		const seen: Array<unknown> = [];
		const on_unhandled = (error: unknown): void => {
			seen.push(error);
		};
		process.on('unhandledRejection', on_unhandled);
		try {
			const program = program_of('vanishing', 'exit 0');
			rmSync(join(bin, 'vanishing'));
			try {
				await program.start([]);
				assert.fail('expected a launch failure');
			} catch (error) {
				assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
			}
			const child = await program_of('fine', 'exit 3').start([]);
			const result = await child.closed;
			assert.deepEqual({ ...result }, { kind: 'exited', code: 3 });
			await sleep(20);
			assert.deepEqual(seen, []);
		} finally {
			process.off('unhandledRejection', on_unhandled);
		}
	});

	test('`stdin.write` copies its bytes, so a later mutation never reaches the child', async () => {
		const child = await program_of('counts', 'tr -d A | wc -c').start([], {
			stdio: ['pipe', 'pipe', 'ignore']
		});
		// past what a pipe takes at once, so Node holds the rest when the call returns
		const data = new Uint8Array(4 * 1024 * 1024).fill(65);
		const written = child.stdin!.write(data);
		data.fill(66);
		assert.isTrue(await written);
		await child.stdin!.close();
		assert.strictEqual((await read_all(child.stdout!)).trim(), '0');
		await child.closed;
	});

	test('`stdin.write` copies through no species a subclass controls', async () => {
		class Species extends Uint8Array {
			static get [Symbol.species](): never {
				throw new Error('species consulted');
			}
		}
		const child = await program_of('cats', 'exec cat').start([], {
			stdio: ['pipe', 'pipe', 'ignore']
		});
		assert.isTrue(await child.stdin!.write(new Species([104, 105])));
		await child.stdin!.close();
		assert.strictEqual(await read_all(child.stdout!), 'hi');
		await child.closed;
	});

	test('`stdin` write after the child ended resolves false, close is safe to repeat', async () => {
		const child = await program_of('nostdin', 'exec 0<&-; exit 0').start([], {
			stdio: ['pipe', 'ignore', 'ignore']
		});
		const stdin = child.stdin!;
		await child.closed;
		assert.strictEqual(await stdin.write('late'), false);
		await stdin.close();
		await stdin.close();
		await assert_rejects_type(() => stdin.write(1 as any));
		await assert_rejects_type(() => stdin.write('a\ud800b'));
	});

	test('a child that closes stdin while we write produces no unhandled error', async () => {
		const errors: Array<unknown> = [];
		const on_error = (e: unknown): void => {
			errors.push(e);
		};
		process.on('uncaughtException', on_error);
		try {
			const child = await program_of('slam', 'exec 0<&-; sleep 0.1').start([], {
				stdio: ['pipe', 'ignore', 'ignore']
			});
			const big = new Uint8Array(1024 * 1024);
			const results: Array<boolean> = [];
			for (let i = 0; i < 4; i++) results.push(await child.stdin!.write(big));
			await child.closed;
			assert.include(results, false);
			await sleep(20);
			assert.deepEqual(errors, []);
		} finally {
			process.off('uncaughtException', on_error);
		}
	});

	test('options are read once and own, with a bad `stdio` refused', async () => {
		const program = program_of('quiet', 'exit 0');
		let reads = 0;
		const options: any = {
			get stdio() {
				reads++;
				return 'pipe';
			}
		};
		const child = await program.start([], options);
		assert.strictEqual(reads, 1);
		assert.ok(child.stdout);
		await child.closed;
		(Object.prototype as any).stdio = 'pipe';
		try {
			const inherited = await program.start([], {});
			assert.isNull(inherited.stdout);
			await inherited.closed;
		} finally {
			delete (Object.prototype as any).stdio;
		}
		const bad: Array<any> = ['bad', 1, null, ['inherit', 'inherit'], ['inherit', 'inherit', 'x']];
		for (const stdio of bad) await assert_rejects_type(() => program.start([], { stdio }));
		await assert_rejects_type(() => program.start('x' as any));
	});
});

describe('child handle hardening', () => {
	const live_child = async (): Promise<{ child: SpawnChild; program: SpawnProgram }> => {
		const program = program_of('cats', 'exec cat');
		return { child: await program.start([], { stdio: 'pipe' }), program };
	};

	test('the child, its parts, and their prototypes are frozen, and reach no stream', async () => {
		const { child } = await live_child();
		const parts = [child, child.stdin!, child.stdout!, child.stderr!];
		for (const part of parts) {
			assert.ok(Object.isFrozen(part));
			const proto = Object.getPrototypeOf(part);
			assert.ok(Object.isFrozen(proto));
			assert.ok(Object.isFrozen(proto.constructor));
			assert.isNull(Object.getPrototypeOf(proto));
			assert.throws(() => {
				proto.patched = true;
			}, TypeError);
		}
		assert.ok(Object.isFrozen(child.closed));
		// the own keys of a stream wrapper name nothing of the stream underneath
		assert.deepEqual(Object.getOwnPropertyNames(child.stdout!), []);
		assert.deepEqual(Object.getOwnPropertySymbols(child.stdout!), []);
		assert.deepEqual(Object.getOwnPropertyNames(child.stdin!), []);
		await child.stdin!.close();
		await child.closed;
	});

	test('a child and its parts have no `then`, even with a polluted `Object.prototype`', async () => {
		const { child } = await live_child();
		assert.isUndefined((child as any).then);
		(Object.prototype as any).then = function (resolve: (value: string) => void) {
			resolve('forged');
		};
		let resolved: Array<Promise<unknown>>;
		try {
			resolved = [
				Promise.resolve(child),
				Promise.resolve(child.stdin),
				Promise.resolve(child.stdout)
			];
		} finally {
			delete (Object.prototype as any).then;
		}
		assert.strictEqual(await resolved[0], child);
		assert.strictEqual(await resolved[1], child.stdin);
		assert.strictEqual(await resolved[2], child.stdout);
		await child.stdin!.close();
		await child.closed;
	});

	// only a chunk or an object carrying one sees it, as a `then` on all would derail the test's awaits
	const with_polluted_chunk_then = async <T>(fn: () => Promise<T>): Promise<T> => {
		Object.defineProperty(Object.prototype as any, 'then', {
			configurable: true,
			get(this: unknown) {
				const carries_chunk =
					this instanceof Uint8Array ||
					Object.values(this as object).some((value) => value instanceof Uint8Array);
				if (!carries_chunk) return undefined;
				return (resolve: (value: string) => void) => {
					resolve('forged');
				};
			}
		});
		try {
			return await fn();
		} finally {
			delete (Object.prototype as any).then;
		}
	};

	const assert_chunk = (result: IteratorResult<Uint8Array>, expected: string): void => {
		assert.isFalse(result.done);
		assert.instanceOf(result.value, Uint8Array);
		assert.strictEqual(new TextDecoder().decode(result.value), expected);
	};

	test('a buffered chunk comes through a polluted `Object.prototype.then` intact', async () => {
		const child = await program_of('buffered', 'printf hello').start([], { stdio: 'pipe' });
		await child.closed;
		const iterator = child.stdout![Symbol.asyncIterator]();
		assert_chunk(await with_polluted_chunk_then(() => iterator.next()), 'hello');
	});

	test('a chunk a reader waited for comes through a polluted `then` intact', async () => {
		const child = await program_of('gated', 'read -r line; printf hello').start([], {
			stdio: 'pipe'
		});
		const iterator = child.stdout![Symbol.asyncIterator]();
		const result = await with_polluted_chunk_then(async () => {
			const pending = iterator.next();
			await child.stdin!.write('go\n');
			return await pending;
		});
		assert_chunk(result, 'hello');
		await child.closed;
	});

	test("a part's constructor makes no handle", async () => {
		const { child } = await live_child();
		const iterator = child.stdout![Symbol.asyncIterator]();
		for (const part of [child, child.stdin!, child.stdout!, iterator] as Array<any>) {
			const Ctor = Object.getPrototypeOf(part).constructor;
			for (const args of [[], [Symbol('spawn handle'), {}, null, null], [Symbol('x')]]) {
				assert.throws(() => Reflect.construct(Ctor, args), TypeError);
			}
		}
		await child.stdin!.close();
		await child.closed;
	});

	test('no method acts on a borrowed receiver', async () => {
		const { child } = await live_child();
		const iterator = child.stdout![Symbol.asyncIterator]();
		const objects: Record<string, object> = {
			child,
			stdin: child.stdin!,
			stdout: child.stderr!, // the unused iterable, so its one iteration isn't claimed
			iterator
		};
		const seen = new Set<string>();
		const fakes = [{}, { pid: 1 }];
		for (const [kind, object] of Object.entries(objects)) {
			const proto = Object.getPrototypeOf(object);
			const keys: Array<string | symbol> = [
				...Object.getOwnPropertyNames(proto),
				...Object.getOwnPropertySymbols(proto)
			];
			for (const key of keys) {
				if (key === 'constructor') continue;
				const name = `${kind}.${String(key)}`;
				seen.add(name);
				const descriptor = Object.getOwnPropertyDescriptor(proto, key)!;
				const members = [descriptor.value, descriptor.get].filter(Boolean) as Array<
					(...args: Array<unknown>) => unknown
				>;
				for (const fn of members) {
					for (const fake of fakes) {
						try {
							await fn.call(fake, 'SIGTERM');
							assert.fail(`expected a TypeError from ${name}`);
						} catch (error) {
							assert.instanceOf(error, TypeError, name);
						}
					}
				}
			}
		}
		for (const expected of [
			'child.kill',
			'child.Symbol(Symbol.asyncDispose)',
			'stdin.write',
			'stdin.close',
			'stdout.Symbol(Symbol.asyncIterator)',
			'iterator.next',
			'iterator.return'
		]) {
			assert.ok(seen.has(expected), `enumerated ${expected}`);
		}
		await child.stdin!.close();
		await child.closed;
	});

	test('`child.closed` and the stored parts are not reassignable', async () => {
		const { child } = await live_child();
		assert.throws(() => {
			(child as any).closed = Promise.resolve();
		}, TypeError);
		await child.stdin!.close();
		await child.closed;
	});
});

describe('kill', () => {
	test('refuses anything but an allowed name, and never takes a pid', async () => {
		const child = await program_of('longrun', 'exec cat').start([], {
			stdio: ['pipe', 'ignore', 'ignore']
		});
		for (const signal of ['SIGKIL', 'KILL', 'sigterm', 15, 0, -1, child.pid, null, {}] as any) {
			assert.throws(() => child.kill(signal), TypeError);
		}
		await child.stdin!.close();
		await child.closed;
	});

	test('`kill` after exit returns false, and twice sends twice', async () => {
		const ready = join(sandbox.base, 'ready');
		const child = await program_of(
			'traps',
			`trap '' TERM; trap 'exit 9' USR1; touch '${ready}'; while :; do sleep 0.02; done`
		).start([], { stdio: ['pipe', 'ignore', 'ignore'] });
		await poll_until(() => existsSync(ready), 'the traps to be set');
		assert.isTrue(child.kill());
		assert.isTrue(child.kill('SIGUSR1'));
		const result = await child.closed;
		assert.deepEqual({ ...result }, { kind: 'exited', code: 9 });
		assert.isFalse(child.kill());
		assert.isFalse(child.kill('SIGKILL'));
	});

	test('`kill` after the leader exits sends nothing, sparing what is left in its group', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const child = await program_of('leaves', `sleep 100 &\necho $! > '${grandchild_path}'`).start(
			[],
			{ stdio: ['ignore', 'pipe', 'ignore'] }
		);
		const grandchild = await read_pid_when_ready(grandchild_path);
		// Node reaps the child and emits `exit` in one callback, so a gone pid means `exit` ran
		await poll_until(() => !is_process_alive(child.pid), 'the leader to be reaped');
		assert.isFalse(child.kill('SIGKILL'));
		assert.isTrue(is_process_alive(grandchild), 'the group was not signaled');
		await child.closed;
		await assert_gone(grandchild, 'the grandchild, at the drain bound');
	});

	test('`kill` reaches a piped child and the group behind it', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const child = await program_of(
			'leader',
			`sleep 100 &\necho $! > '${grandchild_path}'\nwait`
		).start([], { stdio: 'pipe' });
		const grandchild = await read_pid_when_ready(grandchild_path);
		assert.isTrue(child.kill('SIGKILL'));
		await assert_gone(grandchild, 'the grandchild');
		await child.closed;
	});

	test('a piped child is its own group, an inheriting one stays in the terminal session', async () => {
		const piped = await program_of('groups', 'ps -o pid=,pgid=,sid= -p $$').start([], {
			stdio: ['ignore', 'pipe', 'ignore']
		});
		const line = (await read_all(piped.stdout!)).trim().split(/\s+/);
		assert.strictEqual(line[1], line[0], 'the piped child leads its group');
		assert.strictEqual(line[2], line[0], 'in a session of its own');
		await piped.closed;

		const inheriting = await program_of(
			'groups2',
			'ps -o pid=,pgid=,sid= -p $$ > ' + join(sandbox.base, 'in_session')
		).start([], { stdio: ['inherit', 'ignore', 'ignore'] });
		await inheriting.closed;
		const inherited = readFileSync(join(sandbox.base, 'in_session'), 'utf8').trim().split(/\s+/);
		assert.notStrictEqual(inherited[1], inherited[0], 'the inheriting child joined no new group');
	});

	test('`out` leads its own group, and `run` does not', async () => {
		const program = program_of('groups', 'ps -o pid=,pgid= -p $$');
		const output = await program.out([]);
		const [pid, pgid] = output.stdout.trim().split(/\s+/);
		assert.strictEqual(pgid, pid);

		const file = join(sandbox.base, 'run_groups');
		const running = program_of('groups_run', `ps -o pid=,pgid= -p $$ > '${file}'`);
		await running.run([]);
		const [run_pid, run_pgid] = readFileSync(file, 'utf8').trim().split(/\s+/);
		assert.notStrictEqual(run_pgid, run_pid);
	});

	test('dispose stops a child that ignores `SIGTERM`, by `SIGKILL`', async () => {
		const ready = join(sandbox.base, 'ready');
		const child = await program_of(
			'stubborn',
			`trap '' TERM; touch '${ready}'; while :; do sleep 0.02; done`
		).start([], { stdio: ['ignore', 'pipe', 'ignore'] });
		await poll_until(() => existsSync(ready), 'the trap to be set');
		await child[Symbol.asyncDispose]();
		const result = await child.closed;
		assert.deepEqual({ ...result }, { kind: 'signaled', signal: 'SIGKILL' });
	});
});

describe('timeout_ms and signal', () => {
	test('a timeout ends the child, reporting `timed_out` with how it ended', async () => {
		const program = program_of('slow', 'exec sleep 100');
		for (const call of [
			() => program.run([], { timeout_ms: 30 }),
			() => program.out([], { timeout_ms: 30 })
		]) {
			const result: any = await call();
			assert.strictEqual(result.kind, 'timed_out');
			assert.strictEqual(result.signal, 'SIGTERM');
			assert.isNull(result.code);
		}
		const child = await program.start([], { timeout_ms: 30, stdio: 'pipe' });
		const result = await child.closed;
		assert.strictEqual(result.kind, 'timed_out');
	});

	// a stop sent only once the child reports its trap is set, which a timer can't wait for
	const out_stopped_when_ready = (program: SpawnProgram, ready: string): Promise<any> => {
		const controller = new AbortController();
		const call = program.out([], { signal: controller.signal });
		void poll_until(() => existsSync(ready), 'the trap to be set').then(() => {
			controller.abort();
		});
		return call;
	};

	test('a child that exits 0 on `SIGTERM` is still stopped, its code reported', async () => {
		const ready = join(sandbox.base, 'ready');
		const program = program_of(
			'graceful',
			`trap 'exit 0' TERM; touch '${ready}'; while :; do sleep 0.02; done`
		);
		const output = await out_stopped_when_ready(program, ready);
		assert.strictEqual(output.kind, 'aborted');
		assert.strictEqual(output.code, 0);
		assert.isNull(output.signal);
	});

	test('a child ignoring `SIGTERM` is killed after the grace', async () => {
		const ready = join(sandbox.base, 'ready');
		const program = program_of(
			'stubborn',
			`trap '' TERM; touch '${ready}'; while :; do sleep 0.02; done`
		);
		const output = await out_stopped_when_ready(program, ready);
		assert.strictEqual(output.kind, 'aborted');
		assert.strictEqual(output.signal, 'SIGKILL');
	});

	test('a malformed `timeout_ms` is a TypeError and it is read once, own only', async () => {
		const program = program_of('ok', 'exit 0');
		const bad: Array<any> = [0, -1, 1.5, NaN, Infinity, 2147483648, '1', null];
		for (const timeout_ms of bad) {
			await assert_rejects_type(() => program.run([], { timeout_ms }));
			await assert_rejects_type(() => program.out([], { timeout_ms }));
			await assert_rejects_type(() => program.start([], { timeout_ms }));
		}
		// the largest value isn't a 1 ms timer, as `setTimeout` makes of anything past it
		const maximal = await program.out([], { timeout_ms: 2147483647 });
		assert.strictEqual(maximal.kind, 'exited');
		let reads = 0;
		const options: any = {
			get timeout_ms() {
				reads++;
				return reads === 1 ? 2147483647 : 1;
			}
		};
		const read_once = await program.out([], options);
		assert.strictEqual(read_once.kind, 'exited');
		assert.strictEqual(reads, 1);
		(Object.prototype as any).timeout_ms = 1;
		let call: Promise<any>;
		try {
			call = program_of('sleepy', 'exec sleep 0.2').out([], {});
		} finally {
			delete (Object.prototype as any).timeout_ms;
		}
		assert.strictEqual((await call).kind, 'exited');
	});

	test('a signal aborting a run stops it with `aborted`', async () => {
		const program = program_of('slow', 'exec sleep 100');
		const controller = new AbortController();
		const call = program.out([], { signal: controller.signal });
		await sleep(20);
		controller.abort();
		const result = await call;
		assert.strictEqual(result.kind, 'aborted');
		assert.strictEqual(result.kind === 'aborted' ? result.signal : null, 'SIGTERM');
	});

	test('a signal already aborted starts nothing and throws `launch_failed`', async () => {
		const marker = join(sandbox.base, 'marker');
		const program = program_of('marks', `touch '${marker}'`);
		const signal = AbortSignal.abort('why');
		for (const call of [
			() => program.run([], { signal }),
			() => program.out([], { signal }),
			() => program.start([], { signal })
		]) {
			try {
				await call();
				assert.fail('expected a launch failure');
			} catch (error) {
				assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
				assert.strictEqual(error.cause, 'why');
			}
		}
		assert.isFalse(existsSync(marker));
	});

	test('a malformed `signal` is a TypeError, and the listener goes when the child ends', async () => {
		const program = program_of('ok', 'exit 0');
		for (const signal of [1, {}, { aborted: true }, null] as Array<any>) {
			await assert_rejects_type(() => program.out([], { signal }));
		}
		const controller = new AbortController();
		await program.out([], { signal: controller.signal });
		assert.strictEqual(getEventListeners(controller.signal, 'abort').length, 0);
	});

	test('a forged `signal` that takes no listener starts nothing', async () => {
		const marker = join(sandbox.base, 'marker');
		const program = program_of('marks', `touch '${marker}'`);
		// `instanceof` passes through the prototype, and the brand checks never run on the proxy
		const signal: any = new Proxy(Object.create(AbortSignal.prototype), {
			get: (target, key) => {
				if (key === 'aborted') return false;
				if (key === 'addEventListener') {
					return () => {
						throw new Error('forged');
					};
				}
				return Reflect.get(target, key);
			}
		});
		await assert_rejects_type(() => program.run([], { signal }));
		await assert_rejects_type(() => program.out([], { signal }));
		await assert_rejects_type(() => program.start([], { signal }));
		assert.isFalse(existsSync(marker));
	});

	test("a forged `signal` that can't remove its listener doesn't crash the host", async () => {
		const program = program_of('ok', 'exit 0');
		const signal: any = new Proxy(Object.create(AbortSignal.prototype), {
			get: (target, key) => {
				if (key === 'aborted') return false;
				if (key === 'addEventListener') return () => undefined;
				if (key === 'removeEventListener') {
					return () => {
						throw new Error('forged');
					};
				}
				return Reflect.get(target, key);
			}
		});
		const result = await program.out([], { signal });
		assert.strictEqual(result.kind, 'exited');
	});

	test('a stop ends a grandchild holding the pipe, with no hang', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const program = program_of(
			'holder',
			`sleep 100 &\necho $! > '${grandchild_path}'\nexec sleep 100`
		);
		const controller = new AbortController();
		const call = program.out([], { signal: controller.signal });
		const grandchild = await read_pid_when_ready(grandchild_path);
		controller.abort();
		assert.strictEqual((await call).kind, 'aborted');
		await assert_gone(grandchild, 'the grandchild');
	});

	test("a stop's `SIGKILL` reaches a grandchild with redirected pipes after the leader exits", async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const ready = join(sandbox.base, 'ready');
		write_script(
			join(bin, 'ignores_term'),
			`trap '' TERM; echo $$ > '${grandchild_path}'; exec sleep 100`
		);
		const program = program_of(
			'leaves_stubborn',
			`ignores_term </dev/null >/dev/null 2>&1 &
			 until [ -s '${grandchild_path}' ]; do sleep 0.01; done
			 touch '${ready}'
			 exec sleep 100`
		);
		const controller = new AbortController();
		const call = program.out([], { signal: controller.signal });
		await poll_until(() => existsSync(ready), 'the grandchild to start');
		const grandchild = await read_pid_when_ready(grandchild_path);
		controller.abort();
		assert.strictEqual((await call).kind, 'aborted');
		await assert_gone(grandchild, 'the grandchild, at the grace kill');
		await poll_until(
			() => process.listenerCount('SIGTERM') === SIGTERM_LISTENERS,
			'the group to leave tracking'
		);
	});

	test('a stop reaches only the child of an inheriting start, its grandchild surviving', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const ready = join(sandbox.base, 'ready');
		const controller = new AbortController();
		const child = await program_of(
			'holder',
			`sleep 100 &\necho $! > '${grandchild_path}'\ntouch '${ready}'\nexec sleep 100`
		).start([], { stdio: ['inherit', 'pipe', 'pipe'], signal: controller.signal });
		await poll_until(() => existsSync(ready), 'the grandchild to start');
		const grandchild = await read_pid_when_ready(grandchild_path);
		controller.abort();
		const result = await child.closed;
		assert.strictEqual(result.kind, 'aborted');
		assert.isTrue(is_process_alive(grandchild), 'the grandchild outlives a pid-only kill');
		// the iterables still end, by the drain bound, despite the grandchild's open pipe
		await read_all(child.stdout!);
		await read_all(child.stderr!);
	});
});

describe('`out` with a background process', () => {
	test('a grandchild holding the pipe past the bound is killed, the result truncated', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const program = program_of(
			'bg',
			`sleep 100 &\necho $! > '${grandchild_path}'\nprintf text\nexit 2`
		);
		const output = await program.out([]);
		assert.strictEqual(output.kind, 'exited');
		assert.strictEqual(output.kind === 'exited' ? output.code : -1, 2);
		assert.strictEqual(output.stdout, 'text');
		assert.strictEqual(output.truncated, true);
		await assert_gone(await read_pid_when_ready(grandchild_path), 'the grandchild');
	});

	test('a background process that ends inside the bound is not truncated', async () => {
		// waits for the leader to be gone rather than for an amount of time, so it ends well inside
		const program = program_of(
			'quickbg',
			`(while kill -0 $$ 2>/dev/null; do sleep 0.01; done; printf late) &\nprintf early`
		);
		const output = await program.out([]);
		assert.strictEqual(output.stdout, 'earlylate');
		assert.strictEqual(output.truncated, false);
	});

	test('the cap kills the whole group', async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const program = program_of(
			'spews',
			`sleep 100 &\necho $! > '${grandchild_path}'\nprintf 123456\nwait`
		);
		const output = await program.out([], { max_bytes: 5 });
		assert.strictEqual(output.truncated, true);
		await assert_gone(await read_pid_when_ready(grandchild_path), 'the grandchild');
	});
});

describe('cleanup when the host exits', () => {
	let module_url: string;

	beforeEach(() => {
		module_url = new URL(join(paths.lib, 'spawn.ts'), 'file:').href;
	});

	const run_fixture = (
		program: Spawn,
		name: string,
		body: string,
		timeout_ms: number = 8000
	): Promise<any> => {
		const path = join(sandbox.base, `${name}.ts`);
		writeFileSync(path, `import {spawn} from ${JSON.stringify(module_url)};\n${body}\n`);
		chmodSync(path, 0o644);
		return program.program('node').out([path], { timeout_ms });
	};

	const write_child_scripts = (): { sleeper: string; ready: string } => {
		const ready = join(sandbox.base, 'child_pid');
		const sleeper = join(bin, 'sleeper');
		write_script(sleeper, `echo $$ > '${ready}'\nexec sleep 100`);
		return { sleeper, ready };
	};

	test('a piped child is killed when the host calls `process.exit`', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'exit_call',
			`const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.exit(3);`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 3);
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	test('an inheriting child is killed by pid when the host hits an uncaught exception', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'throw',
			`const c = await spawn.program('sleeper').start([], {stdio: ['inherit', 'ignore', 'ignore']});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 throw new Error('host boom');`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.include(result.stderr, 'host boom');
		await assert_gone(await read_pid_when_ready(ready), 'the inheriting child');
	});

	test('an uncaughtException handler that exits still cleans up', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'throw_handled',
			`process.on('uncaughtException', () => process.exit(5));
			 const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 throw new Error('x');`
		);
		assert.strictEqual(result.code, 5);
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	test('an exit hook failure neither crashes nor hangs the exit', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'hook_fails',
			`const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 Set.prototype[Symbol.iterator] = function () { throw new Error('patched'); };
			 process.exit(6);`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 6);
		assert.notInclude(result.stderr, 'Error');
		// the patched `Set` kept the hook from reaching the child, so the test ends it
		process.kill(await read_pid_when_ready(ready), 'SIGKILL');
	});

	test('a host killed by a signal kills a piped child, and the signal still ends it', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'signal',
			`const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 await new Promise(() => {});`
		);
		assert.strictEqual(result.kind, 'signaled');
		assert.strictEqual(result.signal, 'SIGTERM');
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	test("another listener decides the signal's fate, and the exit hook still covers the child", async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'signal_handled',
			`process.on('SIGTERM', () => setTimeout(() => process.exit(4), 20));
			 const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 await new Promise(() => {});`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 4);
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	test('a signal another listener handles leaves the child running', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'signal_kept',
			`let heard = false;
			 process.on('SIGTERM', () => { heard = true; });
			 const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 while (!heard) await new Promise(r=>setTimeout(r,5));
			 const outcome = await Promise.race([c.closed.then(() => 'closed'), new Promise(r=>setTimeout(()=>r('running'),300))]);
			 console.log(outcome);
			 process.exit(0);`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.stdout.trim(), 'running');
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	test('a `process.once` handler finishes its work, its exit code standing, with a group running', async () => {
		const { ready } = write_child_scripts();
		const result = await run_fixture(
			spawn,
			'signal_once',
			`process.once('SIGTERM', () => setTimeout(() => { console.log('graceful done'); process.exit(4); }, 50));
			 const c = await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 await new Promise(() => {});`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 4);
		assert.include(result.stdout, 'graceful done');
		await assert_gone(await read_pid_when_ready(ready), 'the piped child');
	});

	const start_inheriting_and_signal = (signal: string, ready: string): string =>
		`await spawn.program('sleeper').start([], {stdio: 'inherit'});
		 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
		 process.kill(process.pid, ${JSON.stringify(signal)});
		 await new Promise(() => {});`;

	// the host inherits too, as a host in a group would have its group killed when it ends,
	// reaching the child whether or not the signal was passed on
	const run_ungrouped_fixture = async (name: string, body: string) => {
		const path = join(sandbox.base, `${name}.ts`);
		writeFileSync(path, `import {spawn} from ${JSON.stringify(module_url)};\n${body}\n`);
		const host = await spawn.program('node').start([path], { stdio: 'inherit' });
		return host.closed;
	};

	test("a host's `SIGTERM` reaches an inheriting child, and still ends the host", async () => {
		const { ready } = write_child_scripts();
		const result = await run_ungrouped_fixture(
			'term_inheriting',
			start_inheriting_and_signal('SIGTERM', ready)
		);
		assert.strictEqual(result.kind, 'signaled');
		assert.strictEqual(result.kind === 'signaled' ? result.signal : null, 'SIGTERM');
		await assert_gone(await read_pid_when_ready(ready), 'the inheriting child');
	});

	test('a `SIGTERM` passes down a chain of inheriting hosts', async () => {
		const { ready } = write_child_scripts();
		const middle = join(sandbox.base, 'middle.ts');
		writeFileSync(
			middle,
			`import {spawn} from ${JSON.stringify(module_url)};
			 await spawn.program('sleeper').run([]);`
		);
		const result = await run_ungrouped_fixture(
			'term_chain',
			`await spawn.program('node').start([${JSON.stringify(middle)}], {stdio: 'inherit'});
			 while (!(await import('node:fs')).existsSync(${JSON.stringify(ready)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 await new Promise(() => {});`
		);
		assert.strictEqual(result.kind, 'signaled');
		await assert_gone(await read_pid_when_ready(ready), 'the end of the chain');
	});

	for (const signal of ['SIGINT', 'SIGHUP']) {
		test(`a host's \`${signal}\` isn't sent on to an inheriting child`, async () => {
			const { ready } = write_child_scripts();
			const result = await run_ungrouped_fixture(
				`signal_${signal}`,
				start_inheriting_and_signal(signal, ready)
			);
			assert.strictEqual(result.kind, 'signaled');
			const pid = await read_pid_when_ready(ready);
			await sleep(100);
			assert.isTrue(is_process_alive(pid), 'expected the child to be left running');
		});
	}

	test('another listener decides a `SIGTERM` with an inheriting child running', async () => {
		const { ready } = write_child_scripts();
		const result = await run_ungrouped_fixture(
			'term_inheriting_handled',
			`process.on('SIGTERM', () => setTimeout(() => process.exit(4), 20));
			 ${start_inheriting_and_signal('SIGTERM', ready)}`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.kind === 'exited' ? result.code : null, 4);
		await assert_gone(await read_pid_when_ready(ready), 'the inheriting child');
	});

	test('two copies of the module each kill their own group, and the signal still ends the host', async () => {
		const first = join(sandbox.base, 'first_pid');
		const second = join(sandbox.base, 'second_pid');
		write_script(join(bin, 'first'), `echo $$ > '${first}'\nexec sleep 100`);
		write_script(join(bin, 'second'), `echo $$ > '${second}'\nexec sleep 100`);
		const result = await run_fixture(
			spawn,
			'two_copies',
			`const other = (await import(${JSON.stringify(`${module_url}?second_copy`)})).spawn;
			 const a = await spawn.program('first').start([], {stdio: 'pipe'});
			 const b = await other.program('second').start([], {stdio: 'pipe'});
			 const fs = await import('node:fs');
			 while (!fs.existsSync(${JSON.stringify(first)}) || !fs.existsSync(${JSON.stringify(second)})) await new Promise(r=>setTimeout(r,5));
			 process.kill(process.pid, 'SIGTERM');
			 await new Promise(() => {});`
		);
		assert.strictEqual(result.kind, 'signaled');
		assert.strictEqual(result.signal, 'SIGTERM');
		await assert_gone(await read_pid_when_ready(first), "the first copy's child");
		await assert_gone(await read_pid_when_ready(second), "the second copy's child");
	});

	test('output past the buffer stays bounded after the leader exits, with or without a reader', async () => {
		// a byte budget, not a duration: past the limit the stream stops whatever the machine's speed,
		// with the leader's own output past the buffer so the stream is paused when Node resumes it
		const result = await run_fixture(
			spawn,
			'bounded',
			`const sh = spawn.program('sh');
			 const flood = 'dd if=/dev/zero bs=65536 count=17 2>/dev/null; head -c 256M /dev/zero & sleep 0.5; exit 0';
			 const unread = await sh.start(['-c', flood], {stdio: ['ignore', 'pipe', 'ignore']});
			 await unread.closed;
			 const behind = await sh.start(['-c', flood], {stdio: ['inherit', 'pipe', 'ignore']});
			 behind.stdout[Symbol.asyncIterator]();
			 await behind.closed;
			 console.log(process.memoryUsage().arrayBuffers);
			 process.exit(0);`,
			20_000
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 0, result.stderr);
		assert.isBelow(Number(result.stdout.trim()), 64 * 1024 * 1024);
	});

	test('a reader behind at the drain bound keeps the host until it has read everything', async () => {
		// an inheriting child, whose background process isn't killed, so nothing ends the pipe
		// and only the stream being ref'd again keeps the host alive for the read
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const result = await run_fixture(
			spawn,
			'behind',
			`const c = await spawn.program('sh').start(['-c', ${JSON.stringify(`sleep 100 & echo $! > '${grandchild_path}'; dd if=/dev/zero bs=65536 count=17 2>/dev/null`)}], {stdio: ['inherit', 'pipe', 'ignore']});
			 const iterator = c.stdout[Symbol.asyncIterator]();
			 await c.closed;
			 let total = 0;
			 for (let next = await iterator.next(); !next.done; next = await iterator.next()) total += next.value.length;
			 console.log(total);`,
			20_000
		);
		await read_pid_when_ready(grandchild_path);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 0, result.stderr);
		assert.strictEqual(result.stdout.trim(), String(17 * 65536));
	});

	test('a start that runs out of file descriptors is `launch_failed`', async () => {
		const path = join(sandbox.base, 'emfile.ts');
		writeFileSync(
			path,
			`import {spawn} from ${JSON.stringify(module_url)};
			 import {openSync} from 'node:fs';
			 const sh = spawn.program('sh');
			 const held = [];
			 try { for (;;) held.push(openSync('/dev/null', 'r')); } catch {}
			 let outcome;
			 try {
			 	const c = await sh.start(['-c', 'exit 0'], {stdio: 'pipe'});
			 	outcome = 'started';
			 	await c.closed;
			 } catch (error) {
			 	outcome = error.name + ':' + error.kind;
			 }
			 console.log(outcome);
			 process.exit(0);`
		);
		// the limit is scoped to this one child, which fills its own table and nothing else's
		const result = await spawn
			.program('sh')
			.out(['-c', `ulimit -n 128 && exec node '${path}'`], { timeout_ms: 20_000 });
		assert.strictEqual(result.kind, 'exited', result.stderr);
		assert.strictEqual(result.stdout.trim(), 'SpawnError:launch_failed', result.stderr);
	});

	test("a reader that stops reading can't hold the host open past the drain bound", async () => {
		const grandchild_path = join(sandbox.base, 'grandchild_pid');
		const result = await run_fixture(
			spawn,
			'abandoned',
			`const c = await spawn.program('sh').start(['-c', 'sleep 100 & echo $! > ${grandchild_path}; dd if=/dev/zero bs=65536 count=17 2>/dev/null'], {stdio: ['ignore', 'pipe', 'ignore']});
			 c.stdout[Symbol.asyncIterator]();
			 await c.closed;
			 console.log('settled');`
		);
		assert.strictEqual(result.kind, 'exited');
		assert.strictEqual(result.code, 0);
		assert.include(result.stdout, 'settled');
		await assert_gone(await read_pid_when_ready(grandchild_path), 'the bound kills the grandchild');
	});

	test('a host that does not spawn adds no listener, and the exit hook arrives once', async () => {
		vi.resetModules();
		const before = process.listenerCount('exit');
		const fresh = (await import('#lib/spawn.ts')).spawn;
		assert.strictEqual(process.listenerCount('exit'), before);
		const program = fresh.program('sh');
		await program.out(['-c', 'exit 0']);
		await program.out(['-c', 'exit 0']);
		await program.run(['-c', 'exit 0']);
		assert.strictEqual(process.listenerCount('exit'), before + 1);
		vi.resetModules();
	});

	test('`SIGTERM` is listened for while a child runs, the others while a group does', async () => {
		await poll_until(
			() => process.listenerCount('SIGTERM') === SIGTERM_LISTENERS,
			'earlier stops to finish'
		);
		const before = SIGTERM_LISTENERS;
		const program = program_of('longcat', 'exec cat');
		const child = await program.start([], { stdio: ['pipe', 'ignore', 'ignore'] });
		assert.strictEqual(process.listenerCount('SIGTERM'), before + 1);
		await child.stdin!.close();
		await child.closed;
		assert.strictEqual(process.listenerCount('SIGTERM'), before);
		// an inheriting child adds only the `SIGTERM` one
		const sigint_before = process.listenerCount('SIGINT');
		const inheriting = await program_of('quiet', 'exit 0').start([]);
		assert.strictEqual(process.listenerCount('SIGTERM'), before + 1);
		assert.strictEqual(process.listenerCount('SIGINT'), sigint_before);
		await inheriting.closed;
		assert.strictEqual(process.listenerCount('SIGTERM'), before);
	});
});

const assert_rejects_type = async (fn: () => Promise<unknown>): Promise<void> => {
	try {
		await fn();
	} catch (error) {
		assert.instanceOf(error, TypeError);
		return;
	}
	assert.fail('expected a rejection');
};
