import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	chmodSync,
	existsSync,
	mkdirSync,
	readFileSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';

import { fs, type FsDir } from '#lib/fs.ts';
import { FsError, is_fs_error } from '#lib/fs_error.ts';
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

describe('error kinds', () => {
	test('reading a directory is `is_a_directory`, through a file is `not_a_directory`', async () => {
		mkdirSync(join(sandbox.root, 'd'));
		writeFileSync(join(sandbox.root, 'f.txt'), '');
		await assert_fs_error_rejects(dir.read_text('d'), 'is_a_directory');
		await assert_fs_error_rejects(dir.read_text('f.txt/x'), 'not_a_directory');
		await assert_fs_error_rejects(dir.read_text_if_exists('f.txt/x'), 'not_a_directory');
		await assert_fs_error_rejects(dir.stat('f.txt/x'), 'not_a_directory');
	});

	test('an FsError carries its cause and a message naming the path', async () => {
		const error = await assert_fs_error_rejects(dir.read('missing'), 'not_found');
		assert.instanceOf(error, FsError);
		assert.strictEqual(error.name, 'FsError');
		assert.include(error.message, join(sandbox.root, 'missing'));
		assert.strictEqual((error.cause as { code?: string }).code, 'ENOENT');
	});

	test('open_dir refuses a relative path and a non-directory', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), '');
		try {
			await fs.open_dir('relative/path');
			assert.fail('expected a TypeError');
		} catch (error) {
			assert.instanceOf(error, TypeError);
		}
		await assert_fs_error_rejects(fs.open_dir(join(sandbox.root, 'missing')), 'not_found');
		await assert_fs_error_rejects(fs.open_dir(join(sandbox.root, 'f.txt')), 'not_a_directory');
	});

	test('a removed handle directory is `not_found`', async () => {
		mkdirSync(join(sandbox.root, 'gone'));
		const gone = dir.dir('gone');
		await dir.remove('gone');
		assert.isNull(await gone.stat('x'));
		await assert_fs_error_rejects(gone.write('x', 'x'), 'not_found');
	});
});

describe('not_a_directory', () => {
	beforeEach(() => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
	});

	test('a path through a file is `not_a_directory` on every expected-kind path', async () => {
		for (const path of ['f.txt/x', 'f.txt/x/y']) {
			await assert_fs_error_rejects(dir.stat(path), 'not_a_directory');
			await assert_fs_error_rejects(dir.stat(path, { follow: false }), 'not_a_directory');
			await assert_fs_error_rejects(dir.exists(path), 'not_a_directory');
			await assert_fs_error_rejects(dir.read_if_exists(path), 'not_a_directory');
			await assert_fs_error_rejects(dir.read_text_if_exists(path), 'not_a_directory');
			assert_fs_error_throws(() => dir.stat_sync(path), 'not_a_directory');
			assert_fs_error_throws(() => dir.stat_sync(path, { follow: false }), 'not_a_directory');
			assert_fs_error_throws(() => dir.exists_sync(path), 'not_a_directory');
			assert_fs_error_throws(() => dir.dir(path), 'not_a_directory');
		}
	});

	test('`missing_ok` covers only `not_found`', async () => {
		await assert_fs_error_rejects(dir.remove('f.txt/x', { missing_ok: true }), 'not_a_directory');
		await assert_fs_error_rejects(
			dir.remove('f.txt/x', { missing_ok: true, recursive: true }),
			'not_a_directory'
		);
		await dir.remove('missing/x', { missing_ok: true });
	});
});

// each sync op answers as its async twin: the value, or the error's kind
describe('sync and async parity', () => {
	type Outcome = { value: unknown } | { kind: string };

	const settle = async (fn: () => unknown): Promise<Outcome> => {
		try {
			const value = await fn();
			return { value: value !== null && typeof value === 'object' ? { ...value } : value };
		} catch (error) {
			assert.ok(is_fs_error(error), `expected an FsError, got ${error}`);
			return { kind: error.kind };
		}
	};

	const PATHS = [
		'f.txt',
		'd',
		'',
		'missing',
		'missing/deeper',
		'f.txt/x',
		'f.txt/x/y',
		'f.txt/',
		'd/',
		'missing/',
		'link',
		'link/',
		'dangling',
		'dangling/',
		'out',
		'out/secret.txt',
		'out/missing',
		'up',
		'file_slash',
		'file_slash/x',
		'file_dotdot',
		'file_dotdot/f.txt',
		'../x'
	];

	beforeEach(() => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		mkdirSync(join(sandbox.root, 'd'));
		symlinkSync('f.txt', join(sandbox.root, 'link'));
		symlinkSync('missing.txt', join(sandbox.root, 'dangling'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'out'));
		symlinkSync('../root/f.txt', join(sandbox.root, 'up'));
		// the kernel gives `ENOTDIR` for both, never the file or the root
		symlinkSync('f.txt/', join(sandbox.root, 'file_slash'));
		symlinkSync('f.txt/..', join(sandbox.root, 'file_dotdot'));
	});

	test('stat, exists, and read_text', async () => {
		for (const path of PATHS) {
			for (const follow of [true, false]) {
				assert.deepEqual(
					await settle(() => dir.stat_sync(path, { follow })),
					await settle(() => dir.stat(path, { follow })),
					`stat ${JSON.stringify(path)} follow=${follow}`
				);
			}
			assert.deepEqual(
				await settle(() => dir.exists_sync(path)),
				await settle(() => dir.exists(path)),
				`exists ${JSON.stringify(path)}`
			);
			assert.deepEqual(
				await settle(() => dir.read_text_sync(path)),
				await settle(() => dir.read_text(path)),
				`read_text ${JSON.stringify(path)}`
			);
		}
	});

	test('the table holds the expected answers', async () => {
		const expected: Record<string, Outcome> = {
			'f.txt': { value: true },
			d: { value: true },
			missing: { value: false },
			'missing/deeper': { value: false },
			'f.txt/x': { kind: 'not_a_directory' },
			'f.txt/': { kind: 'not_a_directory' },
			'd/': { value: true },
			link: { value: true },
			'link/': { kind: 'not_a_directory' },
			dangling: { value: false },
			out: { kind: 'outside_root' },
			'out/missing': { kind: 'outside_root' },
			up: { kind: 'outside_root' },
			file_slash: { kind: 'not_a_directory' },
			file_dotdot: { kind: 'not_a_directory' },
			'file_dotdot/f.txt': { kind: 'not_a_directory' },
			'../x': { kind: 'outside_root' }
		};
		for (const [path, outcome] of Object.entries(expected)) {
			assert.deepEqual(await settle(() => dir.exists(path)), outcome, path);
		}
		assert_fs_error_throws(() => dir.dir('file_slash'), 'not_a_directory');
		assert_fs_error_throws(() => dir.dir('file_dotdot'), 'not_a_directory');
	});
});

describe('symlink limits', () => {
	beforeEach(() => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
	});

	test('a symlink loop is `io_error`, as the kernel gives `ELOOP`', async () => {
		symlinkSync('loop_b', join(sandbox.root, 'loop_a'));
		symlinkSync('loop_a', join(sandbox.root, 'loop_b'));
		await assert_fs_error_rejects(dir.read_text('loop_a'), 'io_error');
		await assert_fs_error_rejects(dir.stat('loop_a'), 'io_error');
		await assert_fs_error_rejects(dir.read_text('loop_a/x'), 'io_error');
		assert_fs_error_throws(() => dir.stat_sync('loop_a'), 'io_error');
		assert.strictEqual((await dir.stat('loop_a', { follow: false }))?.kind, 'symlink');
	});

	test('a path resolves through 40 symlinks and fails at 41, the kernel limit', async () => {
		// `l0` -> `f.txt`, and each `l<i>` -> `l<i-1>`, so `l<i>` follows `i + 1` links
		symlinkSync('f.txt', join(sandbox.root, 'l0'));
		for (let i = 1; i <= 40; i++) symlinkSync(`l${i - 1}`, join(sandbox.root, `l${i}`));
		assert.strictEqual(readFileSync(join(sandbox.root, 'l39'), 'utf8'), 'f');
		assert.throws(() => readFileSync(join(sandbox.root, 'l40')), /ELOOP/);
		assert.strictEqual(await dir.read_text('l39'), 'f');
		assert.strictEqual(dir.read_text_sync('l39'), 'f');
		await assert_fs_error_rejects(dir.read_text('l40'), 'io_error');
		assert_fs_error_throws(() => dir.read_text_sync('l40'), 'io_error');
	});
});

describe('error paths', () => {
	test("an error from inside the walk names the requested path, not the walk's", async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		symlinkSync('f.txt/', join(sandbox.root, 'file_slash'));
		symlinkSync('gone/../f.txt', join(sandbox.root, 'through_gone'));
		const requested = join(sandbox.root, 'file_slash');
		for (const error of [
			await assert_fs_error_rejects(dir.read_text('file_slash'), 'not_a_directory'),
			assert_fs_error_throws(() => dir.read_text_sync('file_slash'), 'not_a_directory'),
			await assert_fs_error_rejects(dir.write('file_slash', 'x'), 'not_a_directory')
		]) {
			assert.strictEqual(error.path, requested);
			assert.ok(is_fs_error(error.cause, 'not_a_directory'));
			assert.strictEqual(error.cause.path, join(sandbox.root, 'f.txt'));
		}
		const missing = await assert_fs_error_rejects(dir.read_text('through_gone'), 'not_found');
		assert.strictEqual(missing.path, join(sandbox.root, 'through_gone'));
	});
});

// root reads through any permission, so these only run unprivileged
describe.skipIf(process.getuid?.() === 0)('permission denied', () => {
	const lock = (): void => {
		mkdirSync(join(sandbox.root, 'locked'));
		writeFileSync(join(sandbox.root, 'locked/f.txt'), 'f');
		chmodSync(join(sandbox.root, 'locked'), 0o000);
	};
	afterEach(() => {
		if (existsSync(join(sandbox.root, 'locked'))) chmodSync(join(sandbox.root, 'locked'), 0o755);
	});

	test('the expected-kind paths still throw a permission error', async () => {
		lock();
		await assert_fs_error_rejects(dir.stat('locked/f.txt'), 'permission_denied');
		await assert_fs_error_rejects(dir.exists('locked/f.txt'), 'permission_denied');
		await assert_fs_error_rejects(dir.read_text_if_exists('locked/f.txt'), 'permission_denied');
		await assert_fs_error_rejects(dir.read_if_exists('locked/missing'), 'permission_denied');
		await assert_fs_error_rejects(
			dir.remove('locked/f.txt', { missing_ok: true }),
			'permission_denied'
		);
		await assert_fs_error_rejects(
			dir.write('locked/new.txt', 'x', { create_new: true }),
			'permission_denied'
		);
		await assert_fs_error_rejects(
			dir.mkdir('locked/d', { exists_ok: true, recursive: true }),
			'permission_denied'
		);
		assert_fs_error_throws(() => dir.stat_sync('locked/f.txt'), 'permission_denied');
		assert_fs_error_throws(() => dir.exists_sync('locked/f.txt'), 'permission_denied');
		assert_fs_error_throws(() => dir.read_text_sync('locked/f.txt'), 'permission_denied');
	});

	test('listing an unreadable directory is `permission_denied`', async () => {
		lock();
		await assert_fs_error_rejects(
			(async () => {
				for await (const _ of dir.list('locked'));
			})(),
			'permission_denied'
		);
	});
});

describe('is_fs_error', () => {
	// a stand-in for the class from another copy of Gro
	const other_copy = (): typeof Error => {
		class FsError extends Error {
			kind: string;
			path: string;
			constructor(kind: string, path: string) {
				super(kind);
				this.kind = kind;
				this.path = path;
			}
		}
		return FsError as any;
	};

	test('recognizes an FsError, a subclass, and another copy', () => {
		class SubFsError extends FsError {}
		const OtherFsError = other_copy() as any;
		assert.notStrictEqual(OtherFsError, FsError);
		assert.ok(is_fs_error(new FsError('not_found', '/x')));
		assert.ok(is_fs_error(new SubFsError('not_found', '/x')));
		assert.ok(is_fs_error(new OtherFsError('not_found', '/x')));
	});

	test('matches the kind when given', () => {
		const OtherFsError = other_copy() as any;
		assert.ok(is_fs_error(new FsError('not_found', '/x'), 'not_found'));
		assert.ok(!is_fs_error(new FsError('not_found', '/x'), 'already_exists'));
		assert.ok(is_fs_error(new OtherFsError('outside_root', 'x'), 'outside_root'));
		assert.ok(!is_fs_error(new OtherFsError('outside_root', 'x'), 'not_found'));
	});

	test('rejects other errors and non-errors', () => {
		const error_with_kind = Object.assign(new Error('x'), { kind: 'not_found', path: '/x' });
		assert.ok(!is_fs_error(error_with_kind));
		assert.ok(!is_fs_error({ kind: 'not_found', path: '/x' }, 'not_found'));
		assert.ok(!is_fs_error(null));
		assert.ok(!is_fs_error('FsError'));
	});
});
