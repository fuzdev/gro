import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	renameSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { createRequire, syncBuiltinESMExports } from 'node:module';
import { join } from 'node:path';

import { fs, type FsDir, type FsReadDir } from '#lib/fs.ts';
import {
	assert_fs_error_rejects,
	assert_fs_error_throws,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

let sandbox: FsSandbox;
let dir: FsDir;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	dir = sandbox.dir;
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

describe('lexical escapes', () => {
	test('a path climbing above the handle is `outside_root`', async () => {
		const error = await assert_fs_error_rejects(
			dir.read_text('../outside/secret.txt'),
			'outside_root'
		);
		assert.strictEqual(error.path, '../outside/secret.txt');
		await assert_fs_error_rejects(dir.read_text('a/../../outside/secret.txt'), 'outside_root');
		await assert_fs_error_rejects(dir.stat('..'), 'outside_root');
		await assert_fs_error_rejects(dir.exists('../outside'), 'outside_root');
		await assert_fs_error_rejects(dir.write('../x.txt', 'x'), 'outside_root');
		await assert_fs_error_rejects(dir.rename('a', '../a'), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('../outside'), 'outside_root');
		assert.ok(!existsSync(join(sandbox.base, 'x.txt')));
	});

	test('an absolute path is `outside_root`', async () => {
		await assert_fs_error_rejects(
			dir.read_text(join(sandbox.outside, 'secret.txt')),
			'outside_root'
		);
		await assert_fs_error_rejects(dir.read_text(join(sandbox.root, 'a.txt')), 'outside_root');
		assert_fs_error_throws(() => dir.read_text_sync('/etc/passwd'), 'outside_root');
	});

	test('`..` that stays inside is resolved lexically', async () => {
		await dir.write('b.txt', 'b');
		assert.strictEqual(await dir.read_text('a/../b.txt'), 'b');
		assert.strictEqual(await dir.read_text('./a/./../b.txt'), 'b');
	});

	test('dir refuses to narrow above the handle', () => {
		mkdirSync(join(sandbox.root, 'a'));
		assert_fs_error_throws(() => dir.dir('a/../..'), 'outside_root');
		assert_fs_error_throws(() => dir.dir('..'), 'outside_root');
		assert_fs_error_throws(() => dir.dir('a').dir('..'), 'outside_root');
	});
});

describe('symlink escapes', () => {
	test('a handle whose directory is replaced by a symlink to outside is `outside_root`', async () => {
		renameSync(sandbox.root, join(sandbox.base, 'moved'));
		symlinkSync('outside', sandbox.root);
		await assert_fs_error_rejects(dir.read_text('secret.txt'), 'outside_root');
		await assert_fs_error_rejects(dir.stat('missing'), 'outside_root');
		await assert_fs_error_rejects(dir.read_text_if_exists('missing'), 'outside_root');
		await assert_fs_error_rejects(dir.write('new.txt', 'x'), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('missing'), 'outside_root');
		assert_fs_error_throws(() => dir.read_text_sync('secret.txt'), 'outside_root');
		assert.deepEqual(readdirSync(sandbox.outside), ['secret.txt']);
	});

	test('a symlinked file pointing outside is `outside_root`', async () => {
		symlinkSync(join(sandbox.outside, 'secret.txt'), join(sandbox.root, 'link'));
		await assert_fs_error_rejects(dir.read_text('link'), 'outside_root');
		await assert_fs_error_rejects(dir.read_if_exists('link'), 'outside_root');
		await assert_fs_error_rejects(dir.stat('link'), 'outside_root');
		await assert_fs_error_rejects(dir.write('link', 'overwritten'), 'outside_root');
		await assert_fs_error_rejects(dir.copy('link', 'copy.txt'), 'outside_root');
		assert_fs_error_throws(() => dir.read_text_sync('link'), 'outside_root');
		assert_fs_error_throws(() => dir.exists_sync('link'), 'outside_root');
	});

	test('the link itself stays reachable without following it', async () => {
		symlinkSync(join(sandbox.outside, 'secret.txt'), join(sandbox.root, 'link'));
		assert.strictEqual((await dir.stat('link', { follow: false }))!.kind, 'symlink');
		await dir.rename('link', 'link2');
		await dir.remove('link2');
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
	});

	test('a symlinked directory pointing outside is `outside_root`', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		await assert_fs_error_rejects(dir.read_text('link/secret.txt'), 'outside_root');
		await assert_fs_error_rejects(dir.write('link/new.txt', 'x'), 'outside_root');
		await assert_fs_error_rejects(dir.mkdir('link/a/b', { recursive: true }), 'outside_root');
		await assert_fs_error_rejects(dir.remove('link/secret.txt'), 'outside_root');
		await assert_fs_error_rejects(
			(async () => {
				for await (const _ of dir.list('link'));
			})(),
			'outside_root'
		);
		assert_fs_error_throws(() => dir.dir('link'), 'outside_root');
		assert.ok(!existsSync(join(sandbox.outside, 'new.txt')));
		assert.ok(!existsSync(join(sandbox.outside, 'a')));
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
	});

	test('a missing path under a symlink pointing outside is `outside_root`, not `null`', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		await assert_fs_error_rejects(dir.stat('link/missing'), 'outside_root');
		await assert_fs_error_rejects(dir.read_text_if_exists('link/missing'), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('link/missing'), 'outside_root');
	});

	test('a dangling symlink pointing outside is `outside_root`', async () => {
		symlinkSync(join(sandbox.outside, 'created.txt'), join(sandbox.root, 'dangling'));
		symlinkSync(join(sandbox.outside, 'new_dir', 'x'), join(sandbox.root, 'dangling_deep'));
		await assert_fs_error_rejects(dir.write('dangling', 'x'), 'outside_root');
		await assert_fs_error_rejects(dir.write('dangling_deep', 'x'), 'outside_root');
		await assert_fs_error_rejects(dir.stat('dangling'), 'outside_root');
		assert_fs_error_throws(() => dir.exists_sync('dangling'), 'outside_root');
		assert.ok(!existsSync(join(sandbox.outside, 'created.txt')));
	});

	test('a dangling symlink pointing inside is written through', async () => {
		symlinkSync('target.txt', join(sandbox.root, 'dangling'));
		await dir.write('dangling', 'x');
		assert.strictEqual(await dir.read_text('target.txt'), 'x');
	});

	test('a symlink chain that leaves through its last link is `outside_root`', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'out'));
		symlinkSync('out', join(sandbox.root, 'chain'));
		await assert_fs_error_rejects(dir.read_text('chain/secret.txt'), 'outside_root');
	});

	test('symlinks within the handle work', async () => {
		mkdirSync(join(sandbox.root, 'real'));
		writeFileSync(join(sandbox.root, 'real/f.txt'), 'f');
		symlinkSync('real', join(sandbox.root, 'link'));
		symlinkSync('real/f.txt', join(sandbox.root, 'file_link'));
		assert.strictEqual(await dir.read_text('link/f.txt'), 'f');
		assert.strictEqual(await dir.read_text('file_link'), 'f');
		assert.strictEqual(dir.read_text_sync('link/f.txt'), 'f');
		await dir.write('link/g.txt', 'g');
		assert.strictEqual(await dir.read_text('real/g.txt'), 'g');
	});

	test('a narrowed handle refuses a symlink to its parent', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		writeFileSync(join(sandbox.root, 'top.txt'), 'top');
		symlinkSync('../top.txt', join(sandbox.root, 'sub/up'));
		const sub = dir.dir('sub');
		await assert_fs_error_rejects(sub.read_text('up'), 'outside_root');
		assert.strictEqual(await dir.read_text('sub/up'), 'top');
	});
});

// the rules of `openat2` with `RESOLVE_BENEATH`, which the runtime's native op uses
describe('resolve beneath', () => {
	test('an absolute symlink is `outside_root` even when its target is inside', async () => {
		mkdirSync(join(sandbox.root, 'real'));
		writeFileSync(join(sandbox.root, 'real/f.txt'), 'f');
		symlinkSync(join(sandbox.root, 'real'), join(sandbox.root, 'abs_dir'));
		symlinkSync(join(sandbox.root, 'real/f.txt'), join(sandbox.root, 'abs_file'));
		await assert_fs_error_rejects(dir.read_text('abs_dir/f.txt'), 'outside_root');
		await assert_fs_error_rejects(dir.read_text('abs_file'), 'outside_root');
		await assert_fs_error_rejects(dir.stat('abs_dir'), 'outside_root');
		await assert_fs_error_rejects(dir.write('abs_dir/g.txt', 'g'), 'outside_root');
		assert_fs_error_throws(() => dir.read_text_sync('abs_file'), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('abs_dir/f.txt'), 'outside_root');
		assert_fs_error_throws(() => dir.dir('abs_dir'), 'outside_root');
		// the link itself is still reachable without following it
		assert.strictEqual((await dir.stat('abs_dir', { follow: false }))!.kind, 'symlink');
		assert.ok(!existsSync(join(sandbox.root, 'real/g.txt')));
	});

	test('a relative symlink climbing above the handle is `outside_root` even when it lands back inside', async () => {
		mkdirSync(join(sandbox.root, 'real'));
		writeFileSync(join(sandbox.root, 'real/f.txt'), 'f');
		symlinkSync('../root/real', join(sandbox.root, 'up_and_back'));
		symlinkSync('../root/real/f.txt', join(sandbox.root, 'up_and_back_file'));
		await assert_fs_error_rejects(dir.read_text('up_and_back/f.txt'), 'outside_root');
		await assert_fs_error_rejects(dir.read_text('up_and_back_file'), 'outside_root');
		await assert_fs_error_rejects(dir.exists('up_and_back'), 'outside_root');
		assert_fs_error_throws(() => dir.read_text_sync('up_and_back/f.txt'), 'outside_root');
		assert_fs_error_throws(() => dir.exists_sync('up_and_back_file'), 'outside_root');
	});

	test('a missing path under a climbing symlink is `outside_root`, not `null`', async () => {
		symlinkSync('../root', join(sandbox.root, 'up'));
		await assert_fs_error_rejects(dir.stat('up/missing'), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('up/missing'), 'outside_root');
	});

	test("`..` in a symlink's target resolves from where the link leads, as the kernel walks it", async () => {
		mkdirSync(join(sandbox.root, 'real/sub'), { recursive: true });
		writeFileSync(join(sandbox.root, 'real/x.txt'), 'real');
		writeFileSync(join(sandbox.root, 'x.txt'), 'top');
		symlinkSync('real/sub', join(sandbox.root, 'to_sub'));
		symlinkSync('to_sub/../x.txt', join(sandbox.root, 'via'));
		assert.strictEqual(await dir.read_text('via'), 'real');
		assert.strictEqual(dir.read_text_sync('via'), 'real');
		// while the requested path's own `..` resolves lexically
		assert.strictEqual(await dir.read_text('to_sub/../x.txt'), 'top');
	});
});

describe('trailing slash', () => {
	beforeEach(() => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		mkdirSync(join(sandbox.root, 'real'));
		writeFileSync(join(sandbox.root, 'real/f.txt'), 'f');
		symlinkSync('real', join(sandbox.root, 'dir_link'));
	});

	test('names a directory: a file with a trailing `/` is `not_a_directory`', async () => {
		await assert_fs_error_rejects(dir.stat('a.txt/'), 'not_a_directory');
		await assert_fs_error_rejects(dir.stat('a.txt/.'), 'not_a_directory');
		await assert_fs_error_rejects(dir.read('a.txt/'), 'not_a_directory');
		await assert_fs_error_rejects(dir.read_text_if_exists('a.txt/'), 'not_a_directory');
		await assert_fs_error_rejects(dir.exists('a.txt/'), 'not_a_directory');
		assert_fs_error_throws(() => dir.stat_sync('a.txt/'), 'not_a_directory');
		assert_fs_error_throws(() => dir.exists_sync('a.txt/'), 'not_a_directory');
		assert_fs_error_throws(() => dir.read_text_sync('a.txt/'), 'not_a_directory');
		assert.strictEqual((await dir.stat('real/'))!.kind, 'directory');
		assert.strictEqual(dir.stat_sync('real/')!.kind, 'directory');
		assert.isNull(await dir.stat('missing/'));
		assert.isNull(dir.stat_sync('missing/'));
	});

	test('write refuses a path with a trailing `/`', async () => {
		await assert_fs_error_rejects(dir.write('new.txt/', 'x'), 'is_a_directory');
		await assert_fs_error_rejects(
			dir.write('new.txt/', 'x', { create_new: true }),
			'is_a_directory'
		);
		// the kernel's own kind, since `O_CREAT` with a trailing `/` is refused before the lookup
		await assert_fs_error_rejects(dir.write('a.txt/', 'x'), 'is_a_directory');
		assert.ok(!existsSync(join(sandbox.root, 'new.txt')));
		assert.strictEqual(await dir.read_text('a.txt'), 'a');
	});

	test('stat follows a final symlink with a trailing `/`, even without `follow`', async () => {
		assert.strictEqual((await dir.stat('dir_link', { follow: false }))!.kind, 'symlink');
		assert.strictEqual((await dir.stat('dir_link/', { follow: false }))!.kind, 'directory');
		assert.strictEqual(dir.stat_sync('dir_link/', { follow: false })!.kind, 'directory');
		symlinkSync(sandbox.outside, join(sandbox.root, 'out'));
		await assert_fs_error_rejects(dir.stat('out/', { follow: false }), 'outside_root');
		assert_fs_error_throws(() => dir.stat_sync('out/', { follow: false }), 'outside_root');
	});

	test('remove and rename refuse a symlink with a trailing `/`, as the kernel does', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'out'));
		for (const recursive of [false, true]) {
			await assert_fs_error_rejects(dir.remove('dir_link/', { recursive }), 'not_a_directory');
			await assert_fs_error_rejects(dir.remove('out/', { recursive }), 'not_a_directory');
			await assert_fs_error_rejects(dir.remove('out/.', { recursive }), 'not_a_directory');
		}
		await assert_fs_error_rejects(dir.remove('a.txt/'), 'not_a_directory');
		await assert_fs_error_rejects(dir.rename('dir_link/', 'moved'), 'not_a_directory');
		await assert_fs_error_rejects(dir.rename('out/', 'moved'), 'not_a_directory');
		await assert_fs_error_rejects(dir.rename('a.txt', 'moved/'), 'not_a_directory');
		mkdirSync(join(sandbox.root, 'd'));
		await assert_fs_error_rejects(dir.rename('d', 'out/'), 'not_a_directory');
		assert.ok(existsSync(join(sandbox.root, 'real/f.txt')));
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
		assert.ok(existsSync(join(sandbox.root, 'd')));
		assert.ok(existsSync(join(sandbox.root, 'a.txt')));
		assert.strictEqual((await dir.stat('dir_link', { follow: false }))!.kind, 'symlink');
		assert.strictEqual((await dir.stat('out', { follow: false }))!.kind, 'symlink');
	});

	test('remove and rename act on a directory named with a trailing `/`', async () => {
		await dir.rename('real/', 'moved/');
		assert.strictEqual(await dir.read_text('moved/f.txt'), 'f');
		await dir.remove('moved/', { recursive: true });
		await dir.mkdir('empty');
		await dir.remove('empty/');
		assert.isNull(await dir.stat('moved'));
		assert.isNull(await dir.stat('empty'));
		await dir.remove('missing/', { missing_ok: true });
	});
});

/**
 * The absolute path beneath `base` of a name given as bytes, which needn't be UTF-8.
 */
const to_byte_path = (base: string, name: Uint8Array): Buffer =>
	Buffer.concat([Buffer.from(base + '/'), name]);

const FF = new Uint8Array([0xff]);

describe('names that are not UTF-8', () => {
	// an entry named `\xff`, an absolute link out, which a lossy decode of `\xff/...` never finds
	beforeEach(() => {
		symlinkSync(sandbox.outside, to_byte_path(sandbox.root, FF));
		symlinkSync(Buffer.from([0xff, ...Buffer.from('/secret.txt')]), join(sandbox.root, 'L'));
		symlinkSync(Buffer.from([0xff, ...Buffer.from('/planted.txt')]), join(sandbox.root, 'W'));
	});

	test('a symlink whose target is not UTF-8 is refused, never handed to the kernel', async () => {
		await assert_fs_error_rejects(dir.read_text('L'), 'io_error');
		await assert_fs_error_rejects(dir.read_text_if_exists('L'), 'io_error');
		await assert_fs_error_rejects(dir.stat('L'), 'io_error');
		await assert_fs_error_rejects(dir.write('L', 'overwritten'), 'io_error');
		await assert_fs_error_rejects(dir.write('W', 'planted'), 'io_error');
		await assert_fs_error_rejects(dir.write('W', 'planted', { create_new: true }), 'io_error');
		await assert_fs_error_rejects(dir.mkdir('L/x', { recursive: true }), 'io_error');
		assert_fs_error_throws(() => dir.read_text_sync('L'), 'io_error');
		assert_fs_error_throws(() => dir.stat_sync('W'), 'io_error');
		assert_fs_error_throws(() => dir.dir('L'), 'io_error');
		assert.strictEqual(readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'), 'secret');
		assert.deepEqual(readdirSync(sandbox.outside), ['secret.txt']);
	});

	test('the refusal names the link and its target bytes escaped', async () => {
		const error = await assert_fs_error_rejects(dir.read_text('L'), 'io_error');
		assert.include(error.message, '\\xff/secret.txt');
		assert.strictEqual(error.path, join(sandbox.root, 'L'));
	});

	test('so is one that would stay inside, which the native op follows', async () => {
		mkdirSync(to_byte_path(sandbox.root, new Uint8Array([0xfe])));
		writeFileSync(to_byte_path(sandbox.root, new Uint8Array([0xfe, 0x2f, 0x78])), 'x');
		symlinkSync(Buffer.from([0xfe, 0x2f, 0x78]), join(sandbox.root, 'M'));
		await assert_fs_error_rejects(dir.read_text('M'), 'io_error');
	});

	test('`fs.open_dir` refuses a directory whose real path is not UTF-8', async () => {
		mkdirSync(to_byte_path(sandbox.root, new Uint8Array([0xfe])));
		symlinkSync(Buffer.from([0xfe]), join(sandbox.root, 'R'));
		await assert_fs_error_rejects(fs.open_dir(join(sandbox.root, 'R')), 'io_error');
	});

	test('a symlink itself is still removed and renamed, its target never read', async () => {
		await dir.rename('L', 'L2');
		await dir.remove('L2');
		assert.isNull(await dir.stat('L2', { follow: false }));
	});

	test('a listing holding an entry that is not UTF-8 is refused, never a lossy name', async () => {
		writeFileSync(join(sandbox.root, '\uFFFD'), 'replacement');
		const list = async (): Promise<void> => {
			for await (const _ of dir.list()) {
				// drained
			}
		};
		const error = await assert_fs_error_rejects(list(), 'io_error');
		assert.include(error.message, 'holds an entry named \\xff');
		assert.strictEqual(readFileSync(join(sandbox.root, '\uFFFD'), 'utf8'), 'replacement');
	});
});

describe('forged path segments', () => {
	// a polluted intrinsic that turns the segment `x` into a climb out of the handle
	const EVIL = '../../outside/secret.txt';

	const with_patch = async <T extends object, K extends keyof T>(
		target: T,
		key: K,
		make: (original: T[K]) => T[K],
		fn: () => Promise<void>
	): Promise<void> => {
		const original = target[key];
		target[key] = make(original);
		try {
			await fn();
		} finally {
			target[key] = original;
		}
	};

	let sub: FsReadDir;

	beforeEach(() => {
		mkdirSync(join(sandbox.root, 'sub'));
		symlinkSync('sub', join(sandbox.root, 'link'));
		sub = dir.dir('sub').readonly();
	});

	test("a segment holding `/` from a polluted `split` is refused, at the lexical step and the walk's", async () => {
		await with_patch(
			String.prototype,
			'split',
			(split) =>
				function (this: string, ...args: Array<any>) {
					return this === 'x' ? [EVIL] : split.apply(this, args as any);
				} as any,
			async () => {
				await assert_fs_error_rejects(sub.read_text('x'), 'outside_root');
				await assert_fs_error_rejects(dir.readonly().read_text('x'), 'outside_root');
			}
		);
		// a link target split into a climbing name
		await with_patch(
			String.prototype,
			'split',
			(split) =>
				function (this: string, ...args: Array<any>) {
					return this === 'sub' ? ['../outside'] : split.apply(this, args as any);
				} as any,
			async () => {
				await assert_fs_error_rejects(dir.read_text('link/secret.txt'), 'outside_root');
				assert_fs_error_throws(() => dir.read_text_sync('link/secret.txt'), 'outside_root');
			}
		);
	});

	test('a segment holding `/` from a polluted array iterator or `push` is refused', async () => {
		const proto = Array.prototype as any;
		await with_patch(
			proto,
			Symbol.iterator,
			(iterator: any) =>
				function (this: Array<string>) {
					return this.length === 1 && this[0] === 'x' ? iterator.call([EVIL]) : iterator.call(this);
				},
			async () => {
				await assert_fs_error_rejects(sub.read_text('x'), 'outside_root');
			}
		);
		await with_patch(
			proto,
			'push',
			(push: any) =>
				function (this: Array<unknown>, ...args: Array<unknown>) {
					return push.apply(this, args[0] === 'x' ? [EVIL] : args);
				},
			async () => {
				await assert_fs_error_rejects(sub.read_text('x'), 'outside_root');
			}
		);
	});

	test("a segment holding `/` from the walk's polluted `pop` or `toReversed` is refused", async () => {
		const proto = Array.prototype as any;
		await with_patch(
			proto,
			'pop',
			(pop: any) =>
				function (this: Array<unknown>) {
					const value = pop.call(this);
					return value === 'x' ? EVIL : value;
				},
			async () => {
				await assert_fs_error_rejects(dir.readonly().read_text('link/x'), 'outside_root');
				assert_fs_error_throws(() => dir.readonly().read_text_sync('link/x'), 'outside_root');
			}
		);
		await with_patch(
			proto,
			'toReversed',
			(to_reversed: any) =>
				function (this: Array<unknown>) {
					return to_reversed.call(this).map((s: unknown) => (s === 'x' ? EVIL : s));
				},
			async () => {
				await assert_fs_error_rejects(dir.readonly().read_text('link/x'), 'outside_root');
			}
		);
		assert.strictEqual(readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'), 'secret');
	});
});

describe('builtin exports rebound after load', () => {
	test("`syncBuiltinESMExports` can't rebind the Node functions the module copied at load", async () => {
		const require = createRequire(import.meta.url);
		const fs_promises = require('node:fs/promises');
		const node_fs = require('node:fs');
		const { realpath } = fs_promises;
		const { lstatSync } = node_fs;
		symlinkSync(sandbox.outside, join(sandbox.root, 'abslink'));
		mkdirSync(join(sandbox.root, 'sub'));
		try {
			// every path its own real path, which would skip the walk
			fs_promises.realpath = async (path: string) => path;
			// the absolute link reported as a directory, which would walk through it
			node_fs.lstatSync = (path: string, options: unknown) =>
				lstatSync(path.endsWith('/abslink') ? join(sandbox.root, 'sub') : path, options);
			syncBuiltinESMExports();
			await assert_fs_error_rejects(dir.read_text('abslink/secret.txt'), 'outside_root');
			assert_fs_error_throws(() => dir.read_text_sync('abslink/secret.txt'), 'outside_root');
		} finally {
			fs_promises.realpath = realpath;
			node_fs.lstatSync = lstatSync;
			syncBuiltinESMExports();
		}
	});
});
