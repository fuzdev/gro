import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	lstatSync,
	mkdirSync,
	readdirSync,
	statSync,
	symlinkSync,
	utimesSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';

import type { FsDir } from '#lib/fs.ts';
import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

let sandbox: FsSandbox;
let dir: FsDir;
let secret: string;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	dir = sandbox.dir;
	secret = join(sandbox.outside, 'secret.txt');
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

const assert_type_error_rejects = async (fn: () => Promise<unknown>, label = ''): Promise<void> => {
	try {
		await fn();
	} catch (error) {
		assert.instanceOf(error, TypeError, label);
		return;
	}
	assert.fail(`expected a TypeError ${label}`);
};

const mode_of = (path: string): number => statSync(path).mode & 0o7777;

// links out of the handle, to a directory and a file, climbing and absolute
const link_out_of_root = (): void => {
	symlinkSync('../outside', join(sandbox.root, 'up'));
	symlinkSync(sandbox.outside, join(sandbox.root, 'abs'));
	symlinkSync('../outside/secret.txt', join(sandbox.root, 'out'));
	symlinkSync(secret, join(sandbox.root, 'abs_out'));
};

// every way a path reaches `outside/secret.txt` from the handle, each `outside_root`
const PATHS_OUT = ['../outside/secret.txt', 'up/secret.txt', 'abs/secret.txt', 'out', 'abs_out'];

// the step the platform's `utimes` truncates to, read from the platform: a microsecond
// before libuv 1.52, a nanosecond since
const to_utimes_step_ns = (path: string): bigint => {
	const seconds = 1 + 2 ** -19; // 1.000001907… exactly
	utimesSync(path, seconds, seconds);
	return statSync(path, { bigint: true }).mtimeNs === 1_000_001_000n ? 1000n : 1n;
};

// half the spacing of the doubles near `ms / 1000`, the seconds `utimes` takes, rounded to a ns
const to_half_spacing_ns = (ms: number): number =>
	ms < 1000 ? 0 : Math.round((2 ** (Math.floor(Math.log2(ms / 1000)) - 52) * 1e9) / 2);

describe('set_times', () => {
	const T = 1_700_000_000_123;
	// integer times outside the spans where the seconds double is coarser than the ms double, so
	// `stat` shows each exactly on either step; on a microsecond step, `T` lands a microsecond
	// short unless the seconds aim half a step past the time, and `6_000_000_000_105` unless the
	// whole seconds are split off before the aim, and `5_163_330_732_736` a microsecond late if the
	// aim overshoots half the step by as little as 50ns
	const SAMPLES = [
		0,
		1,
		999,
		1234567,
		1234568,
		5705833,
		8541462,
		T,
		1_794_414_326_818,
		4_102_444_800_000,
		5_163_330_732_736,
		6_000_000_000_105
	];

	test('sets the modification and access times, integer ms exactly', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		for (const ms of SAMPLES) {
			await dir.set_times('f.txt', { mtime_ms: ms, atime_ms: ms + 1 });
			const s = statSync(join(sandbox.root, 'f.txt'));
			assert.strictEqual(s.mtimeMs, ms, `${ms}`);
			assert.strictEqual(s.atimeMs, ms + 1, `${ms}`);
			assert.strictEqual((await dir.stat('f.txt'))!.mtime_ms, ms, `${ms}`);
		}
	});

	test('stores an integer ms exactly on a microsecond step, else within half the seconds double', async () => {
		const path = join(sandbox.root, 'f.txt');
		writeFileSync(path, 'f');
		const step = to_utimes_step_ns(path);
		for (const ms of SAMPLES) {
			await dir.set_times('f.txt', { mtime_ms: ms });
			const error = statSync(path, { bigint: true }).mtimeNs - BigInt(ms) * 1_000_000n;
			const bound = step === 1000n ? 0 : to_half_spacing_ns(ms);
			assert.ok(Math.abs(Number(error)) <= bound, `${ms}: ${error}ns, bound ${bound}ns`);
		}
	});

	test('stores a fractional ms on the step nearest it, within half the seconds double', async () => {
		const path = join(sandbox.root, 'f.txt');
		writeFileSync(path, 'f');
		const step = to_utimes_step_ns(path);
		// each time is a whole ms and `n / 4096` of one, a double exactly at these magnitudes, so
		// the error is measured against the time given, in 4096ths of a nanosecond
		for (const ms of [1234568, 5705833, T, 1_794_414_326_818]) {
			for (const n of [1, 3, 2048, 4095]) {
				await dir.set_times('f.txt', { mtime_ms: ms + n / 4096 });
				const target = (BigInt(ms) * 4096n + BigInt(n)) * 1_000_000n;
				const stored = statSync(path, { bigint: true }).mtimeNs * 4096n;
				const error = Number(stored - target) / 4096;
				const bound = Number(step) / 2 + to_half_spacing_ns(ms);
				assert.ok(Math.abs(error) <= bound, `${ms} + ${n}/4096: ${error}ns, bound ${bound}ns`);
			}
		}
	});

	test('`atime_ms` defaults to `mtime_ms`', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		await dir.set_times('f.txt', { mtime_ms: T });
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).atimeMs, T);
	});

	test('sets a directory and the handle directory itself', async () => {
		mkdirSync(join(sandbox.root, 'd'));
		await dir.set_times('d', { mtime_ms: T });
		await dir.set_times('', { mtime_ms: T + 1 });
		assert.strictEqual(statSync(join(sandbox.root, 'd')).mtimeMs, T);
		assert.strictEqual(statSync(sandbox.root).mtimeMs, T + 1);
	});

	test('follows a final symlink, or with `follow: false` sets the link itself', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		symlinkSync('f.txt', join(sandbox.root, 'l'));
		await dir.set_times('l', { mtime_ms: 1_000 }, { follow: false });
		await dir.set_times('f.txt', { mtime_ms: 2_000 });
		await dir.set_times('l', { mtime_ms: 3_000 });
		assert.strictEqual(lstatSync(join(sandbox.root, 'l')).mtimeMs, 1_000);
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).mtimeMs, 3_000);
		await dir.set_times('l', { mtime_ms: 4_000 }, { follow: false });
		assert.strictEqual(lstatSync(join(sandbox.root, 'l')).mtimeMs, 4_000);
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).mtimeMs, 3_000);
	});

	test('a dangling link is `not_found` followed and set without following', async () => {
		symlinkSync('missing', join(sandbox.root, 'dangling'));
		await assert_fs_error_rejects(dir.set_times('dangling', { mtime_ms: T }), 'not_found');
		await dir.set_times('dangling', { mtime_ms: T }, { follow: false });
		assert.strictEqual(lstatSync(join(sandbox.root, 'dangling')).mtimeMs, T);
		assert.deepEqual(readdirSync(sandbox.root), ['dangling']);
	});

	test('a trailing `/` follows a final symlink even without `follow`, as the kernel does', async () => {
		mkdirSync(join(sandbox.root, 'd'));
		symlinkSync('d', join(sandbox.root, 'l'));
		const link_mtime = lstatSync(join(sandbox.root, 'l')).mtimeMs;
		await dir.set_times('l/', { mtime_ms: T }, { follow: false });
		assert.strictEqual(statSync(join(sandbox.root, 'd')).mtimeMs, T);
		assert.strictEqual(lstatSync(join(sandbox.root, 'l')).mtimeMs, link_mtime);
	});

	test('is confined, a link out `outside_root` unless the link itself is set', async () => {
		link_out_of_root();
		const before = statSync(secret).mtimeMs;
		for (const path of [...PATHS_OUT, secret]) {
			const error = await assert_fs_error_rejects(
				dir.set_times(path, { mtime_ms: T }),
				'outside_root'
			);
			assert.strictEqual(error.path, path);
		}
		await assert_fs_error_rejects(
			dir.set_times('up/secret.txt', { mtime_ms: T }, { follow: false }),
			'outside_root'
		);
		// a trailing `/` follows the link however `follow` is set, so it's confined
		const outside_mtime = statSync(sandbox.outside).mtimeMs;
		await assert_fs_error_rejects(
			dir.set_times('up/', { mtime_ms: T }, { follow: false }),
			'outside_root'
		);
		assert.strictEqual(statSync(sandbox.outside).mtimeMs, outside_mtime);
		await dir.set_times('out', { mtime_ms: T }, { follow: false });
		assert.strictEqual(lstatSync(join(sandbox.root, 'out')).mtimeMs, T);
		assert.strictEqual(statSync(secret).mtimeMs, before);
	});

	test('a missing entry is `not_found`, a path through a file `not_a_directory`', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		const error = await assert_fs_error_rejects(
			dir.set_times('missing', { mtime_ms: T }),
			'not_found'
		);
		assert.strictEqual(error.path, join(sandbox.root, 'missing'));
		await assert_fs_error_rejects(dir.set_times('f.txt/x', { mtime_ms: T }), 'not_a_directory');
		assert.deepEqual(readdirSync(sandbox.root), ['f.txt']);
	});

	test('a time that is not a finite number from 0 to 8.64e15 is a `TypeError`, changing nothing', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		await dir.set_times('f.txt', { mtime_ms: T });
		const bad = [-1, -0.5, NaN, Infinity, -Infinity, 8.64e15 + 1, 1e300, '1', 1n, null];
		for (const value of bad) {
			const label = String(value);
			await assert_type_error_rejects(
				() => dir.set_times('f.txt', { mtime_ms: value as any }),
				label
			);
			await assert_type_error_rejects(
				() => dir.set_times('f.txt', { mtime_ms: T, atime_ms: value as any }),
				label
			);
		}
		for (const times of [null, undefined, 1, 'x', {}]) {
			await assert_type_error_rejects(() => dir.set_times('f.txt', times as any), typeof times);
		}
		const s = statSync(join(sandbox.root, 'f.txt'));
		assert.strictEqual(s.mtimeMs, T);
		assert.strictEqual(s.atimeMs, T);
	});

	test('reads each time once', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		let reads = 0;
		const times = {
			get mtime_ms() {
				return reads++ === 0 ? T : -1;
			}
		};
		await dir.set_times('f.txt', times);
		assert.strictEqual(reads, 1);
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).mtimeMs, T);
	});

	test('reads `atime_ms` once', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		let reads = 0;
		const times = {
			mtime_ms: T,
			get atime_ms() {
				return reads++ === 0 ? T + 1 : 1_000;
			}
		};
		await dir.set_times('f.txt', times);
		assert.strictEqual(reads, 1);
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).atimeMs, T + 1);
	});

	test('accepts both ends of the range', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		await dir.set_times('f.txt', { mtime_ms: 0, atime_ms: 8.64e15 });
		assert.strictEqual(statSync(join(sandbox.root, 'f.txt')).mtimeMs, 0);
	});
});

describe('set_mode', () => {
	test('sets a file and a directory, the umask not applied', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		mkdirSync(join(sandbox.root, 'd'));
		for (const mode of [0o600, 0o755, 0o777, 0o000, 0o644]) {
			await dir.set_mode('f.txt', mode);
			assert.strictEqual(mode_of(join(sandbox.root, 'f.txt')), mode);
			assert.strictEqual((await dir.stat('f.txt'))!.mode, mode);
		}
		await dir.set_mode('d', 0o700);
		assert.strictEqual(mode_of(join(sandbox.root, 'd')), 0o700);
	});

	test('follows a final symlink inside the handle, leaving the link', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		symlinkSync('f.txt', join(sandbox.root, 'l'));
		await dir.set_mode('l', 0o600);
		assert.strictEqual(mode_of(join(sandbox.root, 'f.txt')), 0o600);
		assert.ok(lstatSync(join(sandbox.root, 'l')).isSymbolicLink());
		symlinkSync('missing', join(sandbox.root, 'dangling'));
		await assert_fs_error_rejects(dir.set_mode('dangling', 0o600), 'not_found');
	});

	test('is confined, a link out `outside_root`', async () => {
		link_out_of_root();
		const before = mode_of(secret);
		for (const path of [...PATHS_OUT, secret]) {
			const error = await assert_fs_error_rejects(dir.set_mode(path, 0o777), 'outside_root');
			assert.strictEqual(error.path, path);
		}
		await assert_fs_error_rejects(dir.set_mode('up', 0o777), 'outside_root');
		assert.strictEqual(mode_of(secret), before);
		assert.strictEqual(mode_of(sandbox.outside), mode_of(sandbox.root));
	});

	test('a missing entry is `not_found`, a path through a file `not_a_directory`', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		const error = await assert_fs_error_rejects(dir.set_mode('missing', 0o600), 'not_found');
		assert.strictEqual(error.path, join(sandbox.root, 'missing'));
		await assert_fs_error_rejects(dir.set_mode('f.txt/x', 0o600), 'not_a_directory');
		await assert_fs_error_rejects(dir.set_mode('f.txt/', 0o600), 'not_a_directory');
	});

	test('a mode that is not an integer from 0 to 0o777 is a `TypeError`, changing nothing', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		await dir.set_mode('f.txt', 0o640);
		const bad = [
			0o1000,
			0o1777,
			0o2755,
			0o4755,
			0o7777,
			-1,
			1.5,
			NaN,
			Infinity,
			'755',
			'0o755',
			'u+x',
			null
		];
		for (const mode of bad) {
			await assert_type_error_rejects(() => dir.set_mode('f.txt', mode as any), String(mode));
		}
		assert.strictEqual(mode_of(join(sandbox.root, 'f.txt')), 0o640);
	});
});

describe('the metadata ops on a borrowed receiver', () => {
	test('throw `TypeError` with arguments that would otherwise succeed', async () => {
		const proto = Object.getPrototypeOf(dir);
		const before = statSync(secret);
		const calls: Array<[string, Array<unknown>]> = [
			['set_times', ['secret.txt', { mtime_ms: 1_000 }]],
			['set_mode', ['secret.txt', 0o777]]
		];
		for (const [name, args] of calls) {
			for (const fake of [
				{ path: sandbox.outside },
				Object.assign(Object.create(proto), { path: sandbox.outside })
			]) {
				await assert_type_error_rejects(async () => proto[name].apply(fake, args), name);
			}
		}
		const after = statSync(secret);
		assert.strictEqual(after.mtimeMs, before.mtimeMs);
		assert.strictEqual(after.mode, before.mode);
	});
});
