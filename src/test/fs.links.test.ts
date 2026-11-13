import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	chmodSync,
	lstatSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	readlinkSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';

import { fs, type FsDir } from '#lib/fs.ts';
import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';
import { assert_rejects_type } from './test_helpers.ts';

let sandbox: FsSandbox;
let dir: FsDir;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	dir = sandbox.dir;
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

// a directory link climbing out, an absolute one out, and a link in `outside` for them to reach
const link_out_of_root = (): void => {
	symlinkSync('../outside', join(sandbox.root, 'up'));
	symlinkSync(sandbox.outside, join(sandbox.root, 'abs'));
	symlinkSync('secret.txt', join(sandbox.outside, 'l'));
};

describe('read_link', () => {
	test('returns the target as stored, wherever it points', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		const targets: Record<string, string> = {
			in: 'a.txt',
			climbing: '../outside/secret.txt',
			absolute: join(sandbox.outside, 'secret.txt'),
			absolute_in: join(sandbox.root, 'a.txt'),
			dangling: 'missing/x',
			dotted: './a.txt/'
		};
		for (const [name, target] of Object.entries(targets)) {
			symlinkSync(target, join(sandbox.root, name));
			assert.strictEqual(await dir.read_link(name), target, name);
		}
	});

	test('reads a link beneath a symlinked directory inside the handle', async () => {
		mkdirSync(join(sandbox.root, 'real'));
		symlinkSync('../../outside', join(sandbox.root, 'real/l'));
		symlinkSync('real', join(sandbox.root, 'alias'));
		assert.strictEqual(await dir.read_link('alias/l'), '../../outside');
		assert.strictEqual(await dir.dir('real').read_link('l'), '../../outside');
	});

	test('a read-only handle reads links', async () => {
		symlinkSync('x', join(sandbox.root, 'l'));
		assert.strictEqual(await dir.readonly().read_link('l'), 'x');
	});

	test('an entry that is not a symlink is `io_error`, a missing one `not_found`', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		mkdirSync(join(sandbox.root, 'd'));
		await assert_fs_error_rejects(dir.read_link('f.txt'), 'io_error');
		await assert_fs_error_rejects(dir.read_link('d'), 'io_error');
		await assert_fs_error_rejects(dir.read_link(''), 'io_error');
		const missing = await assert_fs_error_rejects(dir.read_link('missing'), 'not_found');
		assert.strictEqual(missing.path, join(sandbox.root, 'missing'));
		await assert_fs_error_rejects(dir.read_link('missing/l'), 'not_found');
		await assert_fs_error_rejects(dir.read_link('f.txt/l'), 'not_a_directory');
	});

	test('a target that is not UTF-8 is `io_error`, never decoded lossily', async () => {
		symlinkSync(Buffer.from([0x61, 0xff]), join(sandbox.root, 'bad'));
		const error = await assert_fs_error_rejects(dir.read_link('bad'), 'io_error');
		assert.include(error.message, 'a\\xff');
		assert.strictEqual(error.path, join(sandbox.root, 'bad'));
	});

	test('the path to the link is confined', async () => {
		link_out_of_root();
		for (const path of ['../outside/l', join(sandbox.outside, 'l'), 'up/l', 'abs/l']) {
			const error = await assert_fs_error_rejects(dir.read_link(path), 'outside_root');
			assert.strictEqual(error.path, path);
		}
		// the links themselves are read, not followed
		assert.strictEqual(await dir.read_link('up'), '../outside');
		assert.strictEqual(await dir.read_link('abs'), sandbox.outside);
	});

	test('a trailing `/` follows the link, as the kernel does, so a link out is `outside_root`', async () => {
		link_out_of_root();
		mkdirSync(join(sandbox.root, 'd'));
		symlinkSync('d', join(sandbox.root, 'in'));
		await assert_fs_error_rejects(dir.read_link('up/'), 'outside_root');
		await assert_fs_error_rejects(dir.read_link('abs/'), 'outside_root');
		await assert_fs_error_rejects(dir.read_link('in/'), 'io_error');
	});
});

describe('fs.real_path', () => {
	test('follows every symlink, absolute, relative, and out of any handle', async () => {
		mkdirSync(join(sandbox.root, 'a'));
		writeFileSync(join(sandbox.root, 'a/f.txt'), 'f');
		symlinkSync(sandbox.outside, join(sandbox.root, 'abs'));
		symlinkSync('../outside/secret.txt', join(sandbox.root, 'rel'));
		symlinkSync('a', join(sandbox.root, 'to_a'));
		symlinkSync(join(sandbox.root, 'to_a'), join(sandbox.outside, 'back'));
		const cases: Array<[string, string]> = [
			[sandbox.root, sandbox.root],
			[join(sandbox.root, 'a/f.txt'), join(sandbox.root, 'a/f.txt')],
			[join(sandbox.root, 'abs'), sandbox.outside],
			[join(sandbox.root, 'abs/secret.txt'), join(sandbox.outside, 'secret.txt')],
			[join(sandbox.root, 'rel'), join(sandbox.outside, 'secret.txt')],
			[join(sandbox.outside, 'back/f.txt'), join(sandbox.root, 'a/f.txt')],
			// `..` after a symlink is physical, as the kernel resolves it
			[join(sandbox.root, 'abs') + '/../root/to_a', join(sandbox.root, 'a')],
			[join(sandbox.root, 'a') + '/', join(sandbox.root, 'a')]
		];
		for (const [path, expected] of cases) {
			assert.strictEqual(await fs.real_path(path), expected, path);
		}
	});

	test('a missing path or a dangling symlink is `not_found`, naming the path asked for', async () => {
		symlinkSync('missing', join(sandbox.root, 'dangling'));
		for (const path of [join(sandbox.root, 'missing'), join(sandbox.root, 'dangling')]) {
			const error = await assert_fs_error_rejects(fs.real_path(path), 'not_found');
			assert.strictEqual(error.path, path);
		}
	});

	test('a path through a file is `not_a_directory`, a symlink loop `io_error`', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		symlinkSync('loop', join(sandbox.root, 'loop'));
		await assert_fs_error_rejects(fs.real_path(join(sandbox.root, 'a.txt/')), 'not_a_directory');
		await assert_fs_error_rejects(fs.real_path(join(sandbox.root, 'a.txt/b')), 'not_a_directory');
		await assert_fs_error_rejects(fs.real_path(join(sandbox.root, 'loop')), 'io_error');
	});

	test('a real path that is not UTF-8 is `io_error`, never decoded lossily', async () => {
		mkdirSync(Buffer.from(sandbox.root + '/\xfe', 'latin1'));
		writeFileSync(join(sandbox.root, '\uFFFD'), 'replacement');
		symlinkSync(Buffer.from([0xfe]), join(sandbox.root, 'link'));
		const error = await assert_fs_error_rejects(
			fs.real_path(join(sandbox.root, 'link')),
			'io_error'
		);
		assert.strictEqual(error.path, join(sandbox.root, 'link'));
	});

	test.skipIf(process.getuid?.() === 0)(
		'a path through an unsearchable directory is `permission_denied`',
		async () => {
			mkdirSync(join(sandbox.root, 'locked'));
			writeFileSync(join(sandbox.root, 'locked/a.txt'), 'a');
			chmodSync(join(sandbox.root, 'locked'), 0o000);
			try {
				await assert_fs_error_rejects(
					fs.real_path(join(sandbox.root, 'locked/a.txt')),
					'permission_denied'
				);
			} finally {
				chmodSync(join(sandbox.root, 'locked'), 0o755);
			}
		}
	);

	test('a relative path or one that is no string is a `TypeError`, before anything is read', async () => {
		for (const path of [
			'',
			'.',
			'relative',
			'a/../b',
			1,
			null,
			undefined,
			{ toString: () => '/' }
		]) {
			await assert_rejects_type(() => fs.real_path(path as any), String(path));
		}
	});

	test('is an own frozen op of the root', () => {
		assert.ok(Object.isFrozen(fs));
		assert.ok(Object.hasOwn(fs, 'real_path'));
		assert.isNull(Object.getPrototypeOf(fs));
	});
});

describe('symlink', () => {
	test('stores any target verbatim, never resolving it', async () => {
		const targets: Record<string, string> = {
			in: 'a.txt',
			climbing: '../outside/secret.txt',
			absolute: join(sandbox.outside, 'secret.txt'),
			dangling: 'missing/x',
			dotted: './a//b/',
			spaced: ' a\nb '
		};
		for (const [name, target] of Object.entries(targets)) {
			assert.strictEqual(await dir.symlink(name, target), 'created', name);
			assert.strictEqual(readlinkSync(join(sandbox.root, name)), target, name);
			assert.ok(lstatSync(join(sandbox.root, name)).isSymbolicLink(), name);
			assert.strictEqual(await dir.read_link(name), target, name);
		}
	});

	test('a link it made is confined at use like any other', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await dir.symlink('in', 'a.txt');
		await dir.symlink('out', '../outside/secret.txt');
		await dir.symlink('abs_in', join(sandbox.root, 'a.txt'));
		assert.strictEqual(await dir.read_text('in'), 'a');
		await assert_fs_error_rejects(dir.read_text('out'), 'outside_root');
		await assert_fs_error_rejects(dir.write('out', 'overwritten'), 'outside_root');
		await assert_fs_error_rejects(dir.read_text('abs_in'), 'outside_root');
		assert.strictEqual(readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'), 'secret');
	});

	test('an existing entry of any kind is `exists` and left untouched', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		mkdirSync(join(sandbox.root, 'd'));
		symlinkSync('f.txt', join(sandbox.root, 'l'));
		symlinkSync('missing', join(sandbox.root, 'dangling'));
		symlinkSync('../outside', join(sandbox.root, 'up'));
		for (const name of ['f.txt', 'd', 'l', 'dangling', 'up', '', '.', 'd/..']) {
			assert.strictEqual(await dir.symlink(name, 'new'), 'exists', name);
		}
		assert.strictEqual(readFileSync(join(sandbox.root, 'f.txt'), 'utf8'), 'f');
		assert.ok(lstatSync(join(sandbox.root, 'd')).isDirectory());
		assert.strictEqual(readlinkSync(join(sandbox.root, 'l')), 'f.txt');
		assert.strictEqual(readlinkSync(join(sandbox.root, 'dangling')), 'missing');
		assert.strictEqual(readlinkSync(join(sandbox.root, 'up')), '../outside');
		assert.deepEqual(readdirSync(join(sandbox.root, 'd')), []);
		assert.deepEqual(readdirSync(sandbox.root).sort(), ['d', 'dangling', 'f.txt', 'l', 'up']);
		assert.deepEqual(readdirSync(sandbox.outside), ['secret.txt']);
	});

	test('racing creators of one path get one `created` and the rest `exists`, none an error', async () => {
		for (let round = 0; round < 5; round++) {
			const name = `race${round}`;
			const results = await Promise.all(
				Array.from({ length: 16 }, (_, i) => dir.symlink(name, `t${i}`))
			);
			assert.strictEqual(results.filter((r) => r === 'created').length, 1, name);
			assert.strictEqual(results.filter((r) => r === 'exists').length, 15, name);
			assert.strictEqual(readlinkSync(join(sandbox.root, name)), `t${results.indexOf('created')}`);
		}
	});

	test('the parent is confined, and nothing is made outside', async () => {
		link_out_of_root();
		for (const path of ['../x', '../outside/x', join(sandbox.outside, 'x'), 'up/x', 'abs/x']) {
			const error = await assert_fs_error_rejects(dir.symlink(path, 'secret.txt'), 'outside_root');
			assert.strictEqual(error.path, path);
		}
		assert.deepEqual(readdirSync(sandbox.outside).sort(), ['l', 'secret.txt']);
		assert.deepEqual(readdirSync(sandbox.base).sort(), ['outside', 'root']);
	});

	test('a symlinked directory inside the handle is followed to the parent', async () => {
		mkdirSync(join(sandbox.root, 'real'));
		symlinkSync('real', join(sandbox.root, 'alias'));
		assert.strictEqual(await dir.symlink('alias/l', 'x'), 'created');
		assert.strictEqual(readlinkSync(join(sandbox.root, 'real/l')), 'x');
	});

	test('a missing parent is `not_found`, a file parent `not_a_directory`', async () => {
		writeFileSync(join(sandbox.root, 'f.txt'), 'f');
		const missing = await assert_fs_error_rejects(dir.symlink('missing/l', 'x'), 'not_found');
		assert.strictEqual(missing.path, join(sandbox.root, 'missing/l'));
		await assert_fs_error_rejects(dir.symlink('f.txt/l', 'x'), 'not_a_directory');
		assert.deepEqual(readdirSync(sandbox.root), ['f.txt']);
	});

	test('a path naming a directory is a `TypeError`, creating nothing', async () => {
		mkdirSync(join(sandbox.root, 'd'));
		for (const path of ['l/', 'd/l/', 'l/.', 'd/l/..', 'd/']) {
			await assert_rejects_type(() => dir.symlink(path, 'x'), path);
		}
		assert.deepEqual(readdirSync(sandbox.root), ['d']);
		assert.deepEqual(readdirSync(join(sandbox.root, 'd')), []);
	});

	test('a target that is empty, holds a NUL, is a lone surrogate, or is no string is a `TypeError`', async () => {
		for (const target of [
			'',
			'a\0b',
			'\uD800',
			'a\uDC00',
			1,
			null,
			undefined,
			{ toString: () => 'x' }
		]) {
			await assert_rejects_type(
				() => dir.symlink('l', target as any),
				typeof target === 'string' ? JSON.stringify(target) : typeof target
			);
		}
		assert.deepEqual(readdirSync(sandbox.root), []);
	});
});

describe('the link ops on a borrowed receiver', () => {
	test('throw `TypeError` with arguments that would otherwise succeed', async () => {
		symlinkSync('secret.txt', join(sandbox.outside, 'l'));
		const proto = Object.getPrototypeOf(dir);
		const calls: Array<[string, Array<unknown>]> = [
			['read_link', ['l']],
			['symlink', ['planted', 'secret.txt']]
		];
		for (const [name, args] of calls) {
			for (const fake of [
				{ path: sandbox.outside },
				Object.assign(Object.create(proto), { path: sandbox.outside })
			]) {
				await assert_rejects_type(async () => proto[name].apply(fake, args), name);
			}
		}
		assert.deepEqual(readdirSync(sandbox.outside).sort(), ['l', 'secret.txt']);
	});
});
