import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	lstatSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	realpathSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { fs, type DirEntry, type FsDir } from '#lib/fs.ts';
import { FsError } from '#lib/fs_error.ts';
import {
	assert_fs_error_rejects,
	assert_fs_error_throws,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';
import { assert_awaits_as_itself } from './test_helpers.ts';

let sandbox: FsSandbox;
let dir: FsDir;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	dir = sandbox.dir;
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

const collect = async (entries: AsyncIterable<DirEntry>): Promise<Array<DirEntry>> => {
	const result: Array<DirEntry> = [];
	for await (const entry of entries) result.push(entry);
	return result.sort((a, b) => (a.name < b.name ? -1 : 1));
};

describe('fs', () => {
	test('project is the working directory', () => {
		assert.strictEqual(fs.project.path, process.cwd());
		assert.ok(fs.project.exists_sync('package.json'));
	});

	test('open_dir resolves the real path', async () => {
		symlinkSync(sandbox.root, join(sandbox.base, 'link'));
		const opened = await fs.open_dir(join(sandbox.base, 'link'));
		assert.strictEqual(opened.path, sandbox.root);
	});

	test('temp_dir makes a fresh directory and removes it on disposal', async () => {
		let path: string;
		{
			await using temp = await fs.temp_dir('gro-fs-temp-');
			path = temp.path;
			assert.ok(path.startsWith(realpathSync(tmpdir()) + '/gro-fs-temp-'));
			await temp.mkdir('a/b', { recursive: true });
			await temp.write('a/b/c.txt', 'c');
			assert.ok(existsSync(join(path, 'a/b/c.txt')));
		}
		assert.ok(!existsSync(path));
	});

	test('temp_dir refuses a prefix with a separator', async () => {
		try {
			await fs.temp_dir('../x');
			assert.fail('expected a TypeError');
		} catch (error) {
			assert.instanceOf(error, TypeError);
		}
	});

	test('temp_dir with a prefix of `..` or `.` stays directly under the temp directory', async () => {
		const temp_real = realpathSync(tmpdir());
		for (const prefix of ['..', '.', '']) {
			await using temp = await fs.temp_dir(prefix);
			assert.strictEqual(dirname(temp.path), temp_real, prefix);
			assert.ok(temp.path.startsWith(temp_real + '/' + prefix), prefix);
			assert.notStrictEqual(temp.path, temp_real, prefix);
		}
	});
});

describe('read and write', () => {
	test('writes and reads text and bytes', async () => {
		await dir.write('a.txt', 'hello');
		await dir.write('b.bin', new Uint8Array([0, 1, 255]));
		assert.strictEqual(readFileSync(join(sandbox.root, 'a.txt'), 'utf8'), 'hello');
		assert.strictEqual(await dir.read_text('a.txt'), 'hello');
		assert.deepEqual([...(await dir.read('b.bin'))], [0, 1, 255]);
		assert.strictEqual(dir.read_text_sync('a.txt'), 'hello');
	});

	test('read returns a plain `Uint8Array`, not a `Buffer`', async () => {
		await dir.write('a.txt', 'hi');
		const bytes = await dir.read('a.txt');
		assert.strictEqual(Object.getPrototypeOf(bytes), Uint8Array.prototype);
		assert.ok(!Buffer.isBuffer(bytes));
	});

	test('write replaces an existing file', async () => {
		await dir.write('a.txt', 'one');
		await dir.write('a.txt', 'two');
		assert.strictEqual(await dir.read_text('a.txt'), 'two');
	});

	test('write with `create_new` reports `exists` and keeps the file', async () => {
		assert.strictEqual(await dir.write('a.txt', 'one', { create_new: true }), 'created');
		assert.strictEqual(await dir.write('a.txt', 'two', { create_new: true }), 'exists');
		assert.strictEqual(await dir.read_text('a.txt'), 'one');
	});

	test('write sets the mode of a new file', async () => {
		await dir.write('a.txt', 'x', { mode: 0o600 });
		assert.strictEqual((await dir.stat('a.txt'))!.mode, 0o600);
	});

	test('write masks the mode to `0o777`, so it sets no sticky, setuid, or setgid bit', async () => {
		for (const mode of [0o1700, 0o7700, 0o4700]) {
			const name = `m${mode.toString(8)}.txt`;
			await dir.write(name, 'x', { mode });
			assert.strictEqual((await dir.stat(name))!.mode, 0o700, name);
		}
		// the largest mode accepted, its permission bits left to the umask
		await dir.write('max.txt', 'x', { mode: 0o7777 });
		assert.strictEqual((await dir.stat('max.txt'))!.mode & 0o7000, 0);
	});

	test('a write mode that is not an integer from 0 to 0o7777 is a `TypeError`, writing nothing', async () => {
		for (const mode of ['755', 1.5, -1, 0o10000, NaN, null]) {
			try {
				await dir.write('a.txt', 'x', { mode: mode as any });
				assert.fail(`expected a TypeError for ${String(mode)}`);
			} catch (error) {
				assert.instanceOf(error, TypeError, String(mode));
			}
		}
		assert.deepEqual(readdirSync(sandbox.root), []);
	});

	test('the `_if_exists` reads return `null` for a missing file', async () => {
		await dir.write('a.txt', 'a');
		assert.strictEqual(await dir.read_text_if_exists('a.txt'), 'a');
		assert.deepEqual([...(await dir.read_if_exists('a.txt'))!], [97]);
		assert.isNull(await dir.read_text_if_exists('missing.txt'));
		assert.isNull(await dir.read_if_exists('missing.txt'));
		assert.isNull(await dir.read_text_if_exists('missing/deeper.txt'));
	});

	test('reading a missing file throws `not_found` with the absolute path', async () => {
		const error = await assert_fs_error_rejects(dir.read_text('missing.txt'), 'not_found');
		assert.strictEqual(error.path, join(sandbox.root, 'missing.txt'));
		assert_fs_error_throws(() => dir.read_text_sync('missing.txt'), 'not_found');
	});
});

describe('stat, exists, list', () => {
	test('stat reports kind, size, mode, and mtime', async () => {
		await dir.write('a.txt', 'abc');
		await dir.mkdir('d');
		const info = await dir.stat('a.txt');
		assert.ok(info);
		assert.strictEqual(info.kind, 'file');
		assert.strictEqual(info.size, 3);
		assert.strictEqual(info.mtime_ms, lstatSync(join(sandbox.root, 'a.txt')).mtimeMs);
		assert.strictEqual(info.mode, lstatSync(join(sandbox.root, 'a.txt')).mode & 0o7777);
		assert.strictEqual((await dir.stat('d'))!.kind, 'directory');
		assert.strictEqual((await dir.stat(''))!.kind, 'directory');
		assert.deepEqual(dir.stat_sync('a.txt'), info);
	});

	test('stat follows a symlink unless told not to', async () => {
		await dir.write('a.txt', 'abc');
		symlinkSync('a.txt', join(sandbox.root, 'link'));
		assert.strictEqual((await dir.stat('link'))!.kind, 'file');
		assert.strictEqual((await dir.stat('link', { follow: false }))!.kind, 'symlink');
		assert.strictEqual(dir.stat_sync('link', { follow: false })!.kind, 'symlink');
	});

	test('stat and exists report a missing entry without throwing', async () => {
		assert.isNull(await dir.stat('missing'));
		assert.isNull(await dir.stat('missing/deeper'));
		assert.isNull(dir.stat_sync('missing'));
		assert.isFalse(await dir.exists('missing'));
		assert.isFalse(dir.exists_sync('missing'));
		await dir.write('a.txt', '');
		assert.isTrue(await dir.exists('a.txt'));
		assert.isTrue(dir.exists_sync('a.txt'));
	});

	test('a dangling symlink exists only without following', async () => {
		symlinkSync('missing.txt', join(sandbox.root, 'dangling'));
		assert.isFalse(await dir.exists('dangling'));
		assert.strictEqual((await dir.stat('dangling', { follow: false }))!.kind, 'symlink');
	});

	test('list yields names and kinds without following symlinks', async () => {
		await dir.write('a.txt', '');
		await dir.mkdir('d');
		await dir.write('d/inner.txt', '');
		symlinkSync('d', join(sandbox.root, 'link'));
		assert.deepEqual(await collect(dir.list()), [
			{ name: 'a.txt', kind: 'file' },
			{ name: 'd', kind: 'directory' },
			{ name: 'link', kind: 'symlink' }
		]);
		assert.deepEqual(await collect(dir.list('d')), [{ name: 'inner.txt', kind: 'file' }]);
		assert.deepEqual(await collect(dir.list('link')), [{ name: 'inner.txt', kind: 'file' }]);
	});

	test('list of a missing directory or a file throws', async () => {
		await dir.write('a.txt', '');
		await assert_fs_error_rejects(collect(dir.list('missing')), 'not_found');
		await assert_fs_error_rejects(collect(dir.list('a.txt')), 'not_a_directory');
	});
});

describe('mkdir and remove', () => {
	test('mkdir creates a directory, with parents when recursive', async () => {
		await dir.mkdir('a');
		await dir.mkdir('b/c/d', { recursive: true });
		await dir.mkdir('b/c/d', { recursive: true });
		assert.ok(lstatSync(join(sandbox.root, 'a')).isDirectory());
		assert.ok(lstatSync(join(sandbox.root, 'b/c/d')).isDirectory());
		await assert_fs_error_rejects(dir.mkdir('x/y'), 'not_found');
	});

	test('mkdir throws `already_exists` unless `exists_ok` and a directory is there', async () => {
		await dir.mkdir('a');
		await assert_fs_error_rejects(dir.mkdir('a'), 'already_exists');
		await dir.mkdir('a', { exists_ok: true });
		await dir.write('f', '');
		await assert_fs_error_rejects(dir.mkdir('f', { exists_ok: true }), 'already_exists');
	});

	test('remove deletes a file, an empty directory, and a tree when recursive', async () => {
		await dir.write('a.txt', '');
		await dir.mkdir('empty');
		await dir.mkdir('tree/sub', { recursive: true });
		await dir.write('tree/sub/f.txt', '');
		await dir.remove('a.txt');
		await dir.remove('empty');
		await assert_fs_error_rejects(dir.remove('tree'), 'not_empty');
		await dir.remove('tree', { recursive: true });
		assert.deepEqual(await collect(dir.list()), []);
	});

	test('remove of a missing entry throws unless `missing_ok`', async () => {
		await assert_fs_error_rejects(dir.remove('missing'), 'not_found');
		await assert_fs_error_rejects(dir.remove('missing', { recursive: true }), 'not_found');
		await dir.remove('missing', { missing_ok: true });
		await dir.remove('missing', { missing_ok: true, recursive: true });
	});

	test('remove deletes a symlink, not its target', async () => {
		await dir.mkdir('d');
		await dir.write('d/f.txt', '');
		symlinkSync('d', join(sandbox.root, 'link'));
		await dir.remove('link', { recursive: true });
		assert.isTrue(await dir.exists('d/f.txt'));
		assert.isNull(await dir.stat('link', { follow: false }));
	});

	test("remove refuses the handle's own directory", async () => {
		await dir.write('a.txt', 'a');
		await dir.mkdir('sub');
		for (const path of ['', '.', './', 'sub/..']) {
			for (const recursive of [false, true]) {
				const error = await assert_fs_error_rejects(dir.remove(path, { recursive }), 'io_error');
				assert.include(error.message, "the handle's own directory can't be removed");
			}
		}
		await assert_fs_error_rejects(dir.remove('', { missing_ok: true }), 'io_error');
		const sub = dir.dir('sub');
		await assert_fs_error_rejects(sub.remove('', { recursive: true }), 'io_error');
		assert.ok(existsSync(join(sandbox.root, 'a.txt')));
		assert.ok(existsSync(join(sandbox.root, 'sub')));
		// the parent still removes it
		await dir.remove('sub');
		assert.ok(!existsSync(join(sandbox.root, 'sub')));
	});
});

describe('rename and copy', () => {
	test('rename moves an entry, replacing a file at the target', async () => {
		await dir.write('a.txt', 'a');
		await dir.write('b.txt', 'b');
		await dir.rename('a.txt', 'b.txt');
		assert.isFalse(await dir.exists('a.txt'));
		assert.strictEqual(await dir.read_text('b.txt'), 'a');
		await dir.mkdir('d');
		await dir.rename('b.txt', 'd/c.txt');
		assert.strictEqual(await dir.read_text('d/c.txt'), 'a');
		await assert_fs_error_rejects(dir.rename('missing', 'x'), 'not_found');
	});

	test('copy copies a file, and a tree only when recursive', async () => {
		await dir.write('a.txt', 'a');
		await dir.copy('a.txt', 'b.txt');
		assert.strictEqual(await dir.read_text('b.txt'), 'a');
		await dir.mkdir('tree/sub', { recursive: true });
		await dir.write('tree/sub/f.txt', 'f');
		symlinkSync('sub/f.txt', join(sandbox.root, 'tree/link'));
		await assert_fs_error_rejects(dir.copy('tree', 'tree2'), 'is_a_directory');
		await dir.copy('tree', 'tree2', { recursive: true });
		assert.strictEqual(await dir.read_text('tree2/sub/f.txt'), 'f');
		assert.strictEqual((await dir.stat('tree2/link', { follow: false }))!.kind, 'symlink');
		assert.strictEqual(await dir.read_text('tree2/link'), 'f');
	});

	test("rename refuses the handle's own directory at either end", async () => {
		await dir.write('a.txt', 'a');
		await dir.mkdir('empty');
		for (const from of ['', '.']) {
			const error = await assert_fs_error_rejects(dir.rename(from, 'x'), 'io_error');
			assert.include(error.message, "the handle's own directory can't be renamed");
		}
		await assert_fs_error_rejects(dir.rename('a.txt', ''), 'io_error');
		// an empty directory onto the root would replace it
		await assert_fs_error_rejects(dir.rename('empty', '.'), 'io_error');
		await assert_fs_error_rejects(dir.rename('empty', 'empty/..'), 'io_error');
		assert.ok(existsSync(join(sandbox.root, 'a.txt')));
		assert.ok(existsSync(join(sandbox.root, 'empty')));
	});

	test("copy and write onto or from the handle's own directory fail", async () => {
		await dir.write('a.txt', 'a');
		await assert_fs_error_rejects(dir.write('', 'x'), 'is_a_directory');
		await assert_fs_error_rejects(dir.copy('a.txt', ''), 'is_a_directory');
		await assert_fs_error_rejects(dir.copy('', 'b.txt'), 'is_a_directory');
		// a tree into itself
		await assert_fs_error_rejects(dir.copy('', 'sub', { recursive: true }), 'io_error');
		assert.deepEqual(await collect(dir.list()), [{ name: 'a.txt', kind: 'file' }]);
	});

	test('a rename or copy error names the end that caused it', async () => {
		await dir.write('a.txt', 'a');
		for (const op of ['rename', 'copy'] as const) {
			const to_missing = await assert_fs_error_rejects(
				dir[op]('a.txt', 'nodir/b.txt'),
				'not_found'
			);
			assert.strictEqual(to_missing.path, join(sandbox.root, 'nodir/b.txt'));
			assert.include(to_missing.message, join(sandbox.root, 'a.txt'));
			const from_missing = await assert_fs_error_rejects(dir[op]('missing', 'b.txt'), 'not_found');
			assert.strictEqual(from_missing.path, join(sandbox.root, 'missing'));
			const to_not_dir = await assert_fs_error_rejects(
				dir[op]('a.txt', 'a.txt/b.txt'),
				'not_a_directory'
			);
			assert.strictEqual(to_not_dir.path, join(sandbox.root, 'a.txt/b.txt'));
			symlinkSync(sandbox.outside, join(sandbox.root, `out_${op}`));
			const outside = await assert_fs_error_rejects(
				dir[op]('a.txt', `out_${op}/b.txt`),
				'outside_root'
			);
			assert.strictEqual(outside.path, `out_${op}/b.txt`);
		}
	});
});

describe('narrowing', () => {
	test('dir narrows to a subdirectory', async () => {
		await dir.mkdir('sub/deeper', { recursive: true });
		await dir.write('top.txt', 'top');
		const sub = dir.dir('sub');
		assert.strictEqual(sub.path, join(sandbox.root, 'sub'));
		await sub.write('f.txt', 'f');
		assert.strictEqual(await dir.read_text('sub/f.txt'), 'f');
		assert.strictEqual(sub.dir('deeper').path, join(sandbox.root, 'sub/deeper'));
		assert.isNull(await sub.stat('top.txt'));
		await assert_fs_error_rejects(sub.read_text('../top.txt'), 'outside_root');
	});

	test('dir requires an existing directory', async () => {
		await dir.write('f.txt', '');
		assert_fs_error_throws(() => dir.dir('missing'), 'not_found');
		assert_fs_error_throws(() => dir.dir('f.txt'), 'not_a_directory');
	});

	test('dir through a symlink inside the handle narrows to the real path', () => {
		mkdirSync(join(sandbox.root, 'real'));
		symlinkSync('real', join(sandbox.root, 'link'));
		assert.strictEqual(dir.dir('link').path, join(sandbox.root, 'real'));
	});

	test('readonly has no write ops at runtime', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		mkdirSync(join(sandbox.root, 'sub'));
		const read_only = dir.readonly();
		const sub = read_only.dir('sub');
		for (const handle of [read_only, sub]) {
			for (const name of [
				'write',
				'mkdir',
				'remove',
				'rename',
				'copy',
				'set_times',
				'set_mode',
				'symlink',
				'readonly'
			]) {
				assert.ok(!(name in handle), `read-only handle has \`${name}\``);
			}
		}
		assert.strictEqual(await read_only.read_text('a.txt'), 'a');
		assert.strictEqual(read_only.path, dir.path);
	});

	test("a writable handle's write op can't be called on a read-only handle", async () => {
		const read_only = dir.readonly();
		const write = (dir as any).write as (...args: Array<unknown>) => Promise<unknown>;
		try {
			await write.call(read_only, 'a.txt', 'a');
			assert.fail('expected a TypeError');
		} catch (error) {
			assert.instanceOf(error, TypeError);
		}
		assert.ok(!existsSync(join(sandbox.root, 'a.txt')));
	});

	test("a handle's constructor makes no handle", () => {
		mkdirSync(join(sandbox.root, 'sub'));
		const handles = [dir, dir.readonly(), dir.dir('sub'), dir.dir('sub').readonly(), fs.project];
		for (const handle of handles) {
			const Ctor = (handle as any).constructor;
			assert.strictEqual(Ctor, Object.getPrototypeOf(handle).constructor);
			for (const args of [[sandbox.outside], [Symbol('fs handle'), sandbox.outside], []]) {
				try {
					Reflect.construct(Ctor, args);
					assert.fail('expected a TypeError');
				} catch (error) {
					assert.instanceOf(error, TypeError);
				}
			}
		}
		assert.deepEqual(readdirSync(sandbox.outside), ['secret.txt']);
	});

	test('handles and their prototypes are frozen', async () => {
		await dir.write('a.txt', 'a');
		const other = await fs.open_dir(sandbox.root);
		const read_only = dir.readonly();
		assert.ok(Object.isFrozen(fs));
		assert.throws(() => {
			(fs as any).project = read_only;
		}, TypeError);
		for (const handle of [dir, read_only, dir.readonly().dir('.')]) {
			const proto = Object.getPrototypeOf(handle);
			assert.ok(Object.isFrozen(handle));
			for (let p = proto; p !== null; p = Object.getPrototypeOf(p)) {
				assert.ok(Object.isFrozen(p));
				assert.ok(Object.isFrozen(p.constructor));
			}
			assert.throws(() => {
				proto.read_text = async () => 'patched';
			}, TypeError);
			assert.throws(() => {
				(handle as any).read_text = async () => 'patched';
			}, TypeError);
			assert.throws(() => {
				(handle as any).path = sandbox.outside;
			}, TypeError);
			assert.throws(() => {
				proto.constructor.patched = true;
			}, TypeError);
		}
		assert.throws(() => {
			(dir as any).write = async () => undefined;
		}, TypeError);
		assert.strictEqual(await other.read_text('a.txt'), 'a');
		assert.strictEqual(await read_only.read_text('a.txt'), 'a');
		assert.strictEqual(dir.path, sandbox.root);
	});

	// every method and accessor reads a handle's state only through private fields,
	// so one borrowed onto a fake receiver claiming another `path` throws before acting
	test('no method or accessor acts on a borrowed receiver', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		await using temp = await fs.temp_dir('gro-fs-borrow-');
		const handles: Record<string, object> = {
			dir,
			read_only: dir.readonly(),
			narrowed: dir.dir('sub'),
			narrowed_read_only: dir.dir('sub').readonly(),
			temp
		};
		const args = ['secret.txt', 'moved.txt', { recursive: true, missing_ok: true }];
		const seen: Set<string | symbol> = new Set();
		for (const [kind, handle] of Object.entries(handles)) {
			for (
				let proto = Object.getPrototypeOf(handle);
				proto !== null;
				proto = Object.getPrototypeOf(proto)
			) {
				const keys = [...Object.getOwnPropertyNames(proto), ...Object.getOwnPropertySymbols(proto)];
				assert.deepEqual(Object.getOwnPropertyNames(proto.constructor).sort(), [
					'length',
					'name',
					'prototype'
				]);
				assert.deepEqual(Object.getOwnPropertySymbols(proto.constructor), []);
				for (const key of keys) {
					if (key === 'constructor') continue;
					seen.add(key);
					const descriptor = Object.getOwnPropertyDescriptor(proto, key)!;
					const calls = [descriptor.value, descriptor.get, descriptor.set].filter(
						(f): f is (...a: Array<unknown>) => unknown => typeof f === 'function'
					);
					for (const fn of calls) {
						const fakes = [
							{ path: sandbox.outside },
							Object.assign(Object.create(proto), { path: sandbox.outside })
						];
						for (const fake of fakes) {
							const label = `${kind} ${String(key)}`;
							try {
								const result = fn.apply(fake, args);
								await result;
								if (result && typeof result === 'object' && Symbol.asyncIterator in result) {
									for await (const _ of result as AsyncIterable<unknown>);
								}
								assert.fail(`expected a TypeError from ${label}`);
							} catch (error) {
								assert.instanceOf(error, TypeError, label);
							}
							assert.deepEqual(readdirSync(sandbox.outside), ['secret.txt'], label);
							assert.strictEqual(
								readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'),
								'secret',
								label
							);
						}
					}
				}
			}
		}
		for (const key of [
			'readonly',
			'write',
			'remove',
			'list',
			'exists',
			'read_link',
			'set_times',
			'set_mode',
			'symlink',
			Symbol.asyncDispose
		]) {
			assert.ok(seen.has(key), `enumerated ${String(key)}`);
		}
	});

	test('temp_dir handles are frozen too', async () => {
		await using temp = await fs.temp_dir('gro-fs-frozen-');
		assert.ok(Object.isFrozen(temp));
		assert.ok(Object.isFrozen(Object.getPrototypeOf(temp)));
		assert.throws(() => {
			Reflect.construct((temp as any).constructor, [sandbox.outside]);
		}, TypeError);
	});

	test('a handle awaits as itself, even with a polluted `Object.prototype.then`', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		await using temp = await fs.temp_dir('gro-fs-then-');
		for (const handle of [fs.project, dir, dir.readonly(), dir.dir('sub'), temp]) {
			await assert_awaits_as_itself(handle);
		}
	});

	test('`fs` awaits as itself, even with a polluted `Object.prototype.then`', async () => {
		await assert_awaits_as_itself(fs);
	});
});

describe('path arguments', () => {
	test('a path with a NUL byte is a `TypeError`', async () => {
		const ops: Array<() => unknown> = [
			() => dir.stat('a\0b'),
			() => dir.read('a\0b'),
			() => dir.read_text_if_exists('\0'),
			() => dir.write('x\0', 'x', { create_new: true }),
			() => dir.remove('x\0'),
			() => dir.rename('a', 'b\0'),
			() => dir.read_link('l\0'),
			() => dir.set_times('a\0', { mtime_ms: 0 }),
			() => dir.set_mode('a\0', 0o644),
			() => dir.symlink('l\0', 'x'),
			() => fs.open_dir(sandbox.root + '/\0'),
			() => fs.open_dir('relative\0'),
			() => fs.temp_dir('x\0')
		];
		for (const op of ops) {
			try {
				await op();
				assert.fail('expected a TypeError');
			} catch (error) {
				assert.instanceOf(error, TypeError);
				assert.notInclude(error.message, '\0');
			}
		}
		assert.throws(() => dir.stat_sync('a\0b'), TypeError);
		assert.throws(() => dir.dir('a\0b'), TypeError);
	});

	test('a path with a lone surrogate is a `TypeError`, never the U+FFFD entry it encodes as', async () => {
		writeFileSync(join(sandbox.root, '\uFFFD'), 'replacement');
		const ops: Array<() => unknown> = [
			() => dir.read_text('\uD800'),
			() => dir.stat('a/\uDC00'),
			() => dir.write('\uD800', 'x'),
			() => dir.remove('\uD800'),
			() => dir.read_link('\uD800'),
			() => dir.set_times('\uD800', { mtime_ms: 0 }),
			() => dir.set_mode('\uD800', 0o644),
			() => dir.symlink('\uD800', 'x'),
			() => fs.open_dir(sandbox.root + '/\uD800'),
			() => fs.temp_dir('\uD800')
		];
		for (const op of ops) {
			try {
				await op();
				assert.fail('expected a TypeError');
			} catch (error) {
				assert.instanceOf(error, TypeError);
			}
		}
		assert.throws(() => dir.read_text_sync('\uD800'), TypeError);
		assert.strictEqual(readFileSync(join(sandbox.root, '\uFFFD'), 'utf8'), 'replacement');
	});

	test('temp_dir refuses a prefix that is not a string, whatever it claims', async () => {
		const forged = {
			includes: () => false,
			toString: () => '../' + sandbox.base.slice(1) + '/made-'
		};
		// a String object, which passes a check that reads its own `includes`
		// eslint-disable-next-line no-new-wrappers
		for (const prefix of [forged, Object.assign(new String('x'), { toString: () => '../x' }), 1]) {
			try {
				await fs.temp_dir(prefix as any);
				assert.fail('expected a TypeError');
			} catch (error) {
				assert.instanceOf(error, TypeError);
			}
		}
		assert.deepEqual(readdirSync(sandbox.base).sort(), ['outside', 'root']);
	});
});

describe('FsError', () => {
	test('the class and its prototype are frozen, so no holder hooks how ops classify errors', () => {
		assert.ok(Object.isFrozen(FsError));
		assert.ok(Object.isFrozen(FsError.prototype));
		assert.throws(() => {
			Object.defineProperty(FsError, Symbol.hasInstance, { value: () => true });
		}, TypeError);
		assert.throws(() => {
			(FsError.prototype as any).toString = () => 'patched';
		}, TypeError);
		assert.strictEqual(String(new FsError('io_error', '/x')), 'FsError: io_error: /x');
	});
});
