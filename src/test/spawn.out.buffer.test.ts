import { describe, test, assert, beforeAll, beforeEach, vi } from 'vitest';
import { EventEmitter } from 'node:events';

import type { Spawn, SpawnOutOptions, SpawnOutputBytes, SpawnProgram } from '#lib/spawn.ts';

// through a fake child, which delivers exactly the chunks a test names, what a real pipe won't,
// and a `Uint8Array` that records what the module allocates, in place when it loads

class FakeStream extends EventEmitter {
	destroyed = false;

	destroy(): void {
		this.destroyed = true;
	}
}

class FakeChild extends EventEmitter {
	readonly pid = 2 ** 30;
	readonly stdin = null;
	readonly stdout = new FakeStream();
	readonly stderr = new FakeStream();
	readonly kills: Array<string> = [];

	kill(signal: string): boolean {
		this.kills.push(signal);
		return true;
	}
}

let child: FakeChild;
let spawn: Spawn;
let sh: SpawnProgram;
const allocations: Array<number> = [];
let fail_allocation_of: number | null = null;
let allocation_failures = 0;

beforeAll(async () => {
	vi.doMock('node:child_process', () => ({ spawn: () => child }));
	vi.stubGlobal(
		'Uint8Array',
		new Proxy(Uint8Array, {
			construct: (target, args) => {
				if (args[0] === fail_allocation_of) throw new RangeError(`failed ${++allocation_failures}`);
				if (typeof args[0] === 'number') allocations.push(args[0]);
				return Reflect.construct(target, args);
			}
		})
	);
	try {
		({ spawn } = await import('#lib/spawn.ts'));
	} finally {
		vi.unstubAllGlobals();
	}
	sh = spawn.program('sh');
});

beforeEach(() => {
	child = new FakeChild();
	allocations.length = 0;
	fail_allocation_of = null;
	allocation_failures = 0;
});

const to_ended = async <T>(call: Promise<T>): Promise<T> => {
	child.emit('spawn');
	child.emit('exit', 0, null);
	child.emit('close', 0, null);
	return call;
};

// `stdin: 'inherit'` leaves the child ungrouped, so a cap's kill reaches the fake's own `kill`
const capture = (
	stdout: ReadonlyArray<Uint8Array>,
	options?: SpawnOutOptions,
	stderr: ReadonlyArray<Uint8Array> = []
): Promise<SpawnOutputBytes> => {
	const call = sh.out_bytes([], { stdin: 'inherit', ...options });
	for (const chunk of stdout) child.stdout.emit('data', chunk);
	for (const chunk of stderr) child.stderr.emit('data', chunk);
	return to_ended(call);
};

const pattern = (length: number, seed = 0): Buffer =>
	Buffer.from(Uint8Array.from({ length }, (_, i) => (seed + i) % 251));

const assert_owned_bytes = (actual: Uint8Array, expected: Uint8Array): void => {
	assert.strictEqual(Object.getPrototypeOf(actual), Uint8Array.prototype);
	assert.strictEqual(actual.byteOffset, 0);
	assert.strictEqual(actual.byteLength, actual.buffer.byteLength);
	assert.strictEqual(actual.length, expected.length);
	assert.isTrue(Buffer.compare(actual, expected) === 0, 'bytes differ');
};

// each capture's empty array, which every list of allocations starts with
const EMPTY_CAPTURES = [0, 0];

describe('the capture buffer', () => {
	test('an empty stream allocates nothing and returns an empty array of its own', async () => {
		const output = await capture([]);
		assert.deepEqual(allocations, EMPTY_CAPTURES);
		assert_owned_bytes(output.stdout, new Uint8Array());
		assert_owned_bytes(output.stderr, new Uint8Array());
		assert.notStrictEqual(output.stdout.buffer, output.stderr.buffer);
		assert.strictEqual(output.truncated, false);
	});

	test('a tiny capture allocates little under a large cap', async () => {
		const output = await capture([pattern(3)], { max_bytes: 2 ** 30 });
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096, 3]);
		assert_owned_bytes(output.stdout, pattern(3));
	});

	test('filling the first allocation exactly returns it without another', async () => {
		const output = await capture([pattern(4096)]);
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096]);
		assert_owned_bytes(output.stdout, pattern(4096));
	});

	test('one byte past a capacity doubles it and keeps what came before', async () => {
		const output = await capture([pattern(4096), pattern(1, 7)]);
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096, 8192, 4097]);
		assert_owned_bytes(output.stdout, Buffer.concat([pattern(4096), pattern(1, 7)]));
	});

	test('a chunk larger than twice the capacity doubles until it fits', async () => {
		const output = await capture([pattern(10), pattern(20_000, 3)]);
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096, 32768, 20_010]);
		assert_owned_bytes(output.stdout, Buffer.concat([pattern(10), pattern(20_000, 3)]));
	});

	test('many small writes cost their bytes, a chunk kept by value and not retained', async () => {
		const count = 100_000;
		const chunk = Buffer.alloc(1);
		const expected = Buffer.alloc(count);
		const call = sh.out_bytes([], { stdin: 'inherit' });
		for (let i = 0; i < count; i++) {
			// the same memory every time, as what a retained chunk would then all read back
			chunk[0] = expected[i] = i % 251;
			child.stdout.emit('data', chunk);
		}
		const output = await to_ended(call);
		assert_owned_bytes(output.stdout, expected);
		// doublings and the final copy, none per chunk
		assert.deepEqual(allocations, [
			...EMPTY_CAPTURES,
			...[4096, 8192, 16384, 32768, 65536, 131072],
			count
		]);
	});

	test('stdout and stderr fill separate arrays', async () => {
		const output = await capture([pattern(5000)], undefined, [pattern(9, 4)]);
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 8192, 4096, 5000, 9]);
		assert_owned_bytes(output.stdout, pattern(5000));
		assert_owned_bytes(output.stderr, pattern(9, 4));
	});
});

describe('the capacity at the cap', () => {
	test('exactly at a cap between doublings is not truncated', async () => {
		const output = await capture([pattern(3000), pattern(2000, 9)], { max_bytes: 5000 });
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096, 5000]);
		assert_owned_bytes(output.stdout, Buffer.concat([pattern(3000), pattern(2000, 9)]));
		assert.strictEqual(output.truncated, false);
		assert.deepEqual(child.kills, []);
	});

	test('one byte past the cap is truncated at the cap and kills the child', async () => {
		const output = await capture([pattern(5000), pattern(1, 9)], { max_bytes: 5000 });
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 5000]);
		assert_owned_bytes(output.stdout, pattern(5000));
		assert.strictEqual(output.truncated, true);
		assert.deepEqual(child.kills, ['SIGKILL']);
		assert.isTrue(child.stdout.destroyed && child.stderr.destroyed);
	});

	test('a single chunk past the cap keeps only its prefix', async () => {
		const output = await capture([pattern(5001)], { max_bytes: 5000 });
		assert_owned_bytes(output.stdout, pattern(5000));
		assert.strictEqual(output.truncated, true);
		assert.deepEqual(child.kills, ['SIGKILL']);
	});

	test('a cap under the first allocation limits it', async () => {
		const output = await capture([pattern(3)], { max_bytes: 10 });
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 10, 3]);
		assert_owned_bytes(output.stdout, pattern(3));
		assert.strictEqual(output.truncated, false);
	});

	test('a cap of 0 allocates nothing and is truncated by any byte', async () => {
		const output = await capture([pattern(1)], { max_bytes: 0 });
		assert.deepEqual(allocations, EMPTY_CAPTURES);
		assert_owned_bytes(output.stdout, new Uint8Array());
		assert.strictEqual(output.truncated, true);
	});

	test('a stream that hit the cap stops the other from growing', async () => {
		const call = sh.out_bytes([], { stdin: 'inherit', max_bytes: 5000 });
		child.stdout.emit('data', pattern(5001));
		child.stderr.emit('data', pattern(100));
		const output = await to_ended(call);
		assert_owned_bytes(output.stdout, pattern(5000));
		assert_owned_bytes(output.stderr, new Uint8Array());
	});
});

describe('text capture', () => {
	const euro_split_over_capacity = [
		Buffer.alloc(4095, 0x61),
		Buffer.from([0xe2]),
		Buffer.from([0x82, 0xac])
	];

	test('a multi-byte character split across a doubling stays one character', async () => {
		const call = sh.out([], { stdin: 'inherit' });
		for (const chunk of euro_split_over_capacity) child.stdout.emit('data', chunk);
		const output = await to_ended(call);
		assert.strictEqual(output.stdout, 'a'.repeat(4095) + '€');
		// decoded in place, with no copy to trim the spare capacity
		assert.deepEqual(allocations, [...EMPTY_CAPTURES, 4096, 8192]);
	});

	test('a multi-byte character cut by the cap becomes U+FFFD', async () => {
		const call = sh.out([], { stdin: 'inherit', max_bytes: 4096 });
		for (const chunk of euro_split_over_capacity) child.stdout.emit('data', chunk);
		const output = await to_ended(call);
		assert.strictEqual(output.stdout, 'a'.repeat(4095) + '�');
		assert.strictEqual(output.truncated, true);
	});
});

describe('hardening', () => {
	test('patched copying methods are not read after load', async () => {
		const chunks = [pattern(4096), pattern(10, 5)];
		const refuse = (): never => {
			throw new Error('a patched method was called');
		};
		const patched: ReadonlyArray<[any, string]> = [
			[Buffer.prototype, 'copy'],
			[Buffer.prototype, 'subarray'],
			[Buffer.prototype, 'slice'],
			[Uint8Array.prototype, 'set'],
			[Uint8Array.prototype, 'slice'],
			[Uint8Array.prototype, 'subarray']
		];
		const restore = patched.map(([owner, key]) => {
			const had = Object.hasOwn(owner, key);
			const original = owner[key];
			owner[key] = refuse;
			return () => {
				if (had) owner[key] = original;
				else delete owner[key];
			};
		});
		let output;
		try {
			// nothing else runs before this settles, as the fake child's events are synchronous
			output = await capture(chunks, { max_bytes: 5000 });
		} finally {
			for (const fn of restore) fn();
		}
		assert_owned_bytes(output.stdout, Buffer.concat(chunks));
	});
});

describe('`stdin` data', () => {
	test('a launch failure with no streams to write rejects with `launch_failed`', async () => {
		// as `EMFILE` and `ENFILE` leave a child, with no pid and no streams
		const failed: any = Object.assign(new FakeChild(), {
			pid: undefined,
			stdout: null,
			stderr: null
		});
		child = failed;
		const call = sh.out_bytes([], { stdin: new Uint8Array([97]) });
		child.emit('error', new Error('spawn EMFILE'));
		try {
			await call;
			assert.fail('expected launch_failed');
		} catch (error) {
			assert.strictEqual((error as { kind?: unknown }).kind, 'launch_failed', String(error));
		}
	});
});

describe('allocation failure', () => {
	const to_error = (call: Promise<unknown>): Promise<unknown> =>
		call.then(
			() => null,
			(error: unknown) => error
		);

	test('rejects the call and kills the child, rather than throwing from the stream', async () => {
		fail_allocation_of = 8192;
		const rejected = to_error(sh.out_bytes([], { stdin: 'inherit' }));
		child.stdout.emit('data', pattern(4096));
		child.stdout.emit('data', pattern(1, 7));
		child.stdout.emit('data', pattern(1, 8));
		const error = await to_ended(rejected);
		assert.instanceOf(error, RangeError);
		assert.deepEqual(child.kills, ['SIGKILL']);
		assert.isTrue(child.stdout.destroyed && child.stderr.destroyed);
	});

	test('a failure on both streams keeps the first error and kills once', async () => {
		fail_allocation_of = 8192;
		const rejected = to_error(sh.out([], { stdin: 'inherit' }));
		child.stdout.emit('data', pattern(5000));
		child.stderr.emit('data', pattern(5000));
		const error = await to_ended(rejected);
		assert.instanceOf(error, RangeError);
		assert.strictEqual(error.message, 'failed 1');
		assert.strictEqual(allocation_failures, 1);
		assert.deepEqual(child.kills, ['SIGKILL']);
	});

	test('the call stays pending until the child has ended', async () => {
		fail_allocation_of = 8192;
		let settled = false;
		const rejected = to_error(sh.out_bytes([], { stdin: 'inherit' })).finally(() => {
			settled = true;
		});
		child.emit('spawn');
		child.stdout.emit('data', pattern(5000));
		await new Promise((resolve) => setImmediate(resolve));
		assert.isFalse(settled);
		child.emit('exit', null, 'SIGKILL');
		child.emit('close', null, 'SIGKILL');
		assert.instanceOf(await rejected, RangeError);
	});
});
