import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	chmodSync,
	copyFileSync,
	mkdirSync,
	readFileSync,
	realpathSync,
	renameSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

import { fs, type FsReadDir } from '#lib/fs.ts';
import { is_fs_error } from '#lib/fs_error.ts';
import { register_fs_handle, to_fs_handle_resolve } from '#lib/internal/fs_handles.ts';
import { paths } from '#lib/paths.ts';
import { spawn, type SpawnProgram } from '#lib/spawn.ts';
import { is_spawn_error, type SpawnError, type SpawnErrorKind } from '#lib/spawn_error.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

let sandbox: FsSandbox;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

const is_root = process.getuid?.() === 0;

// a native binary that prints its own `argv[0]` with `-c 'printf %s "$0"'`
const NATIVE_SH = realpathSync('/bin/sh');
const PRINT_ARGV0 = ['-c', 'printf %s "$0"'];

const write_native = (path: string): void => {
	mkdirSync(join(path, '..'), { recursive: true });
	copyFileSync(NATIVE_SH, path);
	chmodSync(path, 0o755);
};

const write_script = (path: string, body: string, mode = 0o755): void => {
	mkdirSync(join(path, '..'), { recursive: true });
	writeFileSync(path, `#!/bin/sh\n${body}\n`);
	chmodSync(path, mode);
};

const assert_spawn_error = (fn: () => unknown, kind: SpawnErrorKind): SpawnError => {
	try {
		fn();
	} catch (error) {
		assert.ok(is_spawn_error(error, kind), `expected ${kind}, got ${String(error)}`);
		return error;
	}
	throw new Error(`expected a SpawnError of kind ${kind}`);
};

// both methods agree on everything but `not_found`
const assert_both_refuse = (dir: FsReadDir, name: string, kind: SpawnErrorKind): SpawnError => {
	assert_spawn_error(() => spawn.find_in(dir, name), kind);
	return assert_spawn_error(() => spawn.program_in(dir, name), kind);
};

const assert_not_found = (dir: FsReadDir, name: string): void => {
	assert.isNull(spawn.find_in(dir, name), name);
	assert_spawn_error(() => spawn.program_in(dir, name), 'not_found');
};

describe('resolution beneath a handle', () => {
	test('runs a native program, `name` as given, `path` its real path, `argv[0]` the found path', async () => {
		write_native(join(sandbox.root, 'release/tool'));
		const tool = spawn.program_in(sandbox.dir, './release/skipped/../tool');
		assert.strictEqual(tool.name, './release/skipped/../tool');
		assert.strictEqual(tool.path, join(sandbox.root, 'release/tool'));
		const output = await tool.out(PRINT_ARGV0);
		assert.strictEqual(output.kind, 'exited');
		assert.strictEqual(output.stdout, join(sandbox.root, 'release/tool'));
		// the found path is absolute, so another cwd changes nothing
		const elsewhere = await tool.out(PRINT_ARGV0, { cwd: await fs.open_dir(sandbox.outside) });
		assert.strictEqual(elsewhere.stdout, join(sandbox.root, 'release/tool'));
	});

	test('a narrowed handle resolves beneath itself, so `..` to a sibling is refused', () => {
		write_native(join(sandbox.root, 'target/release/tool'));
		write_native(join(sandbox.root, 'target/debug/tool'));
		const release = sandbox.dir.dir('target/release');
		assert.strictEqual(spawn.program_in(release, 'tool').path, join(release.path, 'tool'));
		// lands on a file that exists, inside the parent handle, but above this one
		assert_both_refuse(release, '../debug/tool', 'not_granted');
		assert_both_refuse(release, '../release/tool', 'not_granted');
	});

	test('a read-only handle works, as running needs no write', async () => {
		write_native(join(sandbox.root, 'tool'));
		const readonly = sandbox.dir.readonly();
		assert.ok(!('write' in readonly));
		const output = await spawn.program_in(readonly, 'tool').out(['-c', 'exit 3']);
		assert.deepEqual(
			{ ...output },
			{
				kind: 'exited',
				code: 3,
				stdout: '',
				stderr: '',
				truncated: false
			}
		);
	});

	test('an absolute or climbing name is `not_granted` before any disk access', () => {
		for (const name of [
			'/bin/sh',
			'/nonexistent/x',
			'../outside/secret.txt',
			'../x',
			'a/../../x'
		]) {
			const error = assert_both_refuse(sandbox.dir, name, 'not_granted');
			assert.ok(is_fs_error(error.cause, 'outside_root'), name);
		}
		// with the handle's directory gone, a name that reaches the disk is a miss,
		// while the lexical refusals stand, so they came first
		const gone = sandbox.dir.dir('.');
		rmSync(sandbox.root, { recursive: true });
		assert_not_found(gone, 'x');
		for (const name of ['/bin/sh', '../x', 'a/../../x']) {
			assert_both_refuse(gone, name, 'not_granted');
		}
	});

	test('a `..` that stays beneath the handle resolves lexically, through a missing directory', () => {
		write_native(join(sandbox.root, 'tool'));
		assert.ok(spawn.find_in(sandbox.dir, 'a/../tool'));
		assert.ok(spawn.find_in(sandbox.dir, 'missing/deeper/../../tool'));
	});

	test('an absolute symlink is `not_granted`, even one pointing back inside', () => {
		write_native(join(sandbox.root, 'tool'));
		write_native(join(sandbox.outside, 'tool'));
		symlinkSync(join(sandbox.root, 'tool'), join(sandbox.root, 'abs_inside'));
		symlinkSync(join(sandbox.outside, 'tool'), join(sandbox.root, 'abs_outside'));
		for (const name of ['abs_inside', 'abs_outside']) {
			const error = assert_both_refuse(sandbox.dir, name, 'not_granted');
			assert.ok(is_fs_error(error.cause, 'outside_root'), name);
			assert.include(error.message, name);
		}
	});

	test('a relative symlink climbing out is `not_granted`, even one landing back inside', () => {
		write_native(join(sandbox.root, 'tool'));
		write_native(join(sandbox.outside, 'tool'));
		symlinkSync('../outside/tool', join(sandbox.root, 'out'));
		symlinkSync('../root/tool', join(sandbox.root, 'back_in'));
		mkdirSync(join(sandbox.root, 'bin'));
		symlinkSync('../../outside', join(sandbox.root, 'bin/dir_out'));
		for (const name of ['out', 'back_in', 'bin/dir_out/tool']) {
			const error = assert_both_refuse(sandbox.dir, name, 'not_granted');
			assert.ok(is_fs_error(error.cause, 'outside_root'), name);
		}
	});

	test('a relative symlink inside is followed, a script through it seeing the link path as `$0`', async () => {
		write_script(join(sandbox.root, 'pkg/bin/tool.sh'), 'printf %s "$0"');
		mkdirSync(join(sandbox.root, '.bin'));
		symlinkSync('../pkg/bin/tool.sh', join(sandbox.root, '.bin/tool'));
		const tool = spawn.program_in(sandbox.dir, '.bin/tool');
		assert.strictEqual(tool.name, '.bin/tool');
		assert.strictEqual(tool.path, join(sandbox.root, 'pkg/bin/tool.sh'));
		const output = await tool.out([], { cwd: await fs.open_dir(sandbox.outside) });
		assert.strictEqual(output.stdout, join(sandbox.root, '.bin/tool'));
		// a native binary through a link runs at its pin, and passes the found path as `argv[0]`
		write_native(join(sandbox.root, 'pkg/bin/native'));
		symlinkSync('../pkg/bin/native', join(sandbox.root, '.bin/native'));
		const native = spawn.program_in(sandbox.dir, '.bin/native');
		assert.strictEqual(native.path, join(sandbox.root, 'pkg/bin/native'));
		assert.strictEqual((await native.out(PRINT_ARGV0)).stdout, join(sandbox.root, '.bin/native'));
	});

	test('a miss is `not_found`: missing, through a file, a directory, a trailing `/` or `/.`', () => {
		write_native(join(sandbox.root, 'tool'));
		mkdirSync(join(sandbox.root, 'a_dir'));
		symlinkSync('missing_target', join(sandbox.root, 'dangling'));
		for (const name of [
			'missing',
			'missing_dir/tool',
			'tool/x',
			'a_dir',
			'a_dir/',
			'.',
			'tool/',
			'tool/.',
			'dangling'
		]) {
			assert_not_found(sandbox.dir, name);
		}
		const error = assert_spawn_error(() => spawn.program_in(sandbox.dir, 'missing'), 'not_found');
		assert.include(error.message, sandbox.root);
	});

	test('a file the kernel would hand to `/bin/sh` is `not_executable`', () => {
		writeFileSync(join(sandbox.root, 'plain'), '#!/bin/sh\nexit 0\n');
		chmodSync(join(sandbox.root, 'plain'), 0o644);
		writeFileSync(join(sandbox.root, 'empty'), '');
		chmodSync(join(sandbox.root, 'empty'), 0o755);
		writeFileSync(join(sandbox.root, 'junk'), 'echo hi\n');
		chmodSync(join(sandbox.root, 'junk'), 0o755);
		write_script(join(sandbox.root, 'x.cmd'), 'exit 0');
		write_script(join(sandbox.root, 'real.bat'), 'exit 0');
		symlinkSync('real.bat', join(sandbox.root, 'link'));
		for (const name of ['plain', 'empty', 'junk', 'x.cmd', 'X.CMD. ', 'link']) {
			assert_both_refuse(sandbox.dir, name, 'not_executable');
		}
	});

	test.skipIf(is_root)(
		"a directory this user can't search is `not_executable` with the permission error, never a miss",
		() => {
			write_native(join(sandbox.root, 'locked/tool'));
			chmodSync(join(sandbox.root, 'locked'), 0o000);
			try {
				for (const name of ['locked/tool', 'locked/missing']) {
					const error = assert_both_refuse(sandbox.dir, name, 'not_executable');
					assert.ok(is_fs_error(error.cause, 'permission_denied'), name);
				}
			} finally {
				chmodSync(join(sandbox.root, 'locked'), 0o755);
			}
		}
	);

	test('a script retargeted after the handle is made is refused at launch', async () => {
		write_script(join(sandbox.root, 'a.sh'), 'exit 0');
		write_script(join(sandbox.root, 'b.sh'), 'exit 1');
		symlinkSync('a.sh', join(sandbox.root, 'run'));
		const run = spawn.program_in(sandbox.dir, 'run');
		assert.strictEqual((await run.out([])).kind, 'exited');
		rmSync(join(sandbox.root, 'run'));
		symlinkSync('b.sh', join(sandbox.root, 'run'));
		try {
			await run.out([]);
			assert.fail('expected launch_failed');
		} catch (error) {
			assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
		}
	});

	test('a native binary keeps its pin when its path is renamed over', async () => {
		write_native(join(sandbox.root, 'tool'));
		const tool = spawn.program_in(sandbox.dir, 'tool');
		const pin = tool.path;
		renameSync(join(sandbox.root, 'tool'), join(sandbox.root, 'moved'));
		assert.strictEqual(tool.path, pin);
		assert.isNull(spawn.find_in(sandbox.dir, 'tool'));
	});
});

describe('the handle and the root', () => {
	test('a `dir` this copy of fs never made is a TypeError, before the name is read', () => {
		write_native(join(sandbox.root, 'tool'));
		const real = sandbox.dir;
		const forged: Array<unknown> = [
			{ path: sandbox.root },
			{ path: '/' },
			Object.create(Object.getPrototypeOf(real), { path: { value: sandbox.root } }),
			Object.freeze({ ...real }),
			{},
			null,
			undefined,
			sandbox.root,
			spawn
		];
		for (const dir of forged) {
			assert.throws(() => spawn.program_in(dir as FsReadDir, 'tool'), TypeError);
			assert.throws(() => spawn.find_in(dir as FsReadDir, 'tool'), TypeError);
			assert.throws(() => spawn.find_in(dir as FsReadDir, '/bin/sh'), TypeError);
			assert.throws(() => spawn.find_in(dir as FsReadDir, 1 as any), TypeError);
		}
	});

	test('a malformed name is a TypeError', () => {
		const names: Array<any> = ['', 'a\0b', 'a\ud800b', undefined, null, 1, {}];
		for (const name of names) {
			assert.throws(() => spawn.program_in(sandbox.dir, name), TypeError);
			assert.throws(() => spawn.find_in(sandbox.dir, name), TypeError);
		}
	});

	test('a narrowed root refuses both methods, before any resolution, even for a name in its set', () => {
		write_native(join(sandbox.root, 'tool'));
		for (const narrowed of [spawn.narrow(['tool']), spawn.narrow(['sh']).narrow([])]) {
			for (const name of ['tool', 'missing', '../x']) {
				assert_spawn_error(() => narrowed.program_in(sandbox.dir, name), 'not_granted');
				assert_spawn_error(() => narrowed.find_in(sandbox.dir, name), 'not_granted');
			}
			// the handle is still checked first
			assert.throws(() => narrowed.find_in({ path: sandbox.root } as FsReadDir, 'tool'), TypeError);
		}
	});

	test('options are read and checked once, before the name resolves', () => {
		assert.throws(() => spawn.find_in(sandbox.dir, 'missing', { pass_env: 1 } as any), TypeError);
		assert.throws(() => spawn.program_in(sandbox.dir, 'missing', { env: 1 } as any), TypeError);
		assert.throws(() => spawn.find_in(sandbox.dir, '../x', [] as any), TypeError);
		let reads = 0;
		const options: any = {
			get pass_env() {
				reads++;
				return [];
			}
		};
		assert.isNull(spawn.find_in(sandbox.dir, 'missing', options));
		assert.strictEqual(reads, 1);
	});

	test('the environment is the base set plus `pass_env`, fixed at creation', async () => {
		const env_path = join(sandbox.root, 'env');
		copyFileSync(realpathSync('/usr/bin/env'), env_path);
		chmodSync(env_path, 0o755);
		process.env.GRO_PROGRAM_IN_PASSED = 'passed';
		process.env.GRO_PROGRAM_IN_HELD = 'held';
		let env: SpawnProgram;
		try {
			env = spawn.program_in(sandbox.dir, 'env', { pass_env: ['GRO_PROGRAM_IN_PASSED'] });
		} finally {
			delete process.env.GRO_PROGRAM_IN_PASSED;
			delete process.env.GRO_PROGRAM_IN_HELD;
		}
		const lines = (await env.out([])).stdout.split('\n');
		assert.include(lines, 'GRO_PROGRAM_IN_PASSED=passed');
		assert.include(lines, 'LC_ALL=C');
		assert.notInclude(lines, 'GRO_PROGRAM_IN_HELD=held');
	});

	test('a factory bound at the root runs what a library builds after it was bound', async () => {
		await using cache = await fs.temp_dir('gro-program-in-');
		const program_in_cache = (name: string): SpawnProgram => spawn.program_in(cache, name);
		// the library's side: build, then ask the factory for the handle
		assert_spawn_error(() => program_in_cache('build/tool'), 'not_found');
		await cache.mkdir('build');
		await cache.write('build/tool', '#!/bin/sh\nprintf built\n', { mode: 0o755 });
		const output = await program_in_cache('build/tool').out([]);
		assert.strictEqual(output.stdout, 'built');
	});

	test('the handle is frozen, and no method acts on a borrowed receiver', async () => {
		write_native(join(sandbox.root, 'tool'));
		const tool = spawn.program_in(sandbox.dir, 'tool');
		assert.ok(Object.isFrozen(tool));
		assert.strictEqual(Object.getPrototypeOf(tool), Object.getPrototypeOf(spawn.program('sh')));
		const proto = Object.getPrototypeOf(spawn);
		for (const key of ['program_in', 'find_in']) {
			const fn = proto[key] as (...args: Array<unknown>) => unknown;
			for (const fake of [{}, Object.create(proto)]) {
				// args that pass every check, so only the receiver can throw
				assert.throws(() => fn.call(fake, sandbox.dir, 'tool'), TypeError, undefined, key);
			}
		}
	});
});

describe('the fs handle registry', () => {
	test('every kind of handle fs makes is registered', async () => {
		write_native(join(sandbox.root, 'sub/tool'));
		await using temp = await fs.temp_dir();
		await temp.write('tool', '#!/bin/sh\n', { mode: 0o755 });
		const handles: Record<string, FsReadDir> = {
			open_dir: sandbox.dir,
			dir: sandbox.dir.dir('sub'),
			readonly: sandbox.dir.readonly(),
			readonly_dir: sandbox.dir.readonly().dir('sub'),
			dir_readonly: sandbox.dir.dir('sub').readonly(),
			temp_dir: temp,
			temp_dir_dir: temp.dir('.'),
			project: fs.project,
			project_readonly: fs.project.readonly()
		};
		for (const [label, handle] of Object.entries(handles)) {
			assert.ok(Object.isFrozen(handle), label);
			const resolve = to_fs_handle_resolve(handle);
			assert.strictEqual(resolve('.').found_path, handle.path, label);
		}
		assert.ok(spawn.find_in(handles.dir_readonly!, 'tool'));
		assert.ok(spawn.find_in(temp, 'tool'));
	});

	test('a handle registers once, so no later registration replaces its resolver', () => {
		assert.throws(() => register_fs_handle(sandbox.dir, () => assert.fail('replaced')), TypeError);
		const forged = { path: '/' };
		register_fs_handle(forged, () => assert.fail('unused'));
		assert.throws(() => register_fs_handle(forged, () => assert.fail('unused')), TypeError);
	});

	test('the resolver answers by the handle, not its `path`', () => {
		write_native(join(sandbox.root, 'tool'));
		const resolve = to_fs_handle_resolve(sandbox.dir);
		const resolution = resolve('tool');
		assert.deepEqual(
			{ ...resolution },
			{
				found_path: join(sandbox.root, 'tool'),
				op_path: join(sandbox.root, 'tool')
			}
		);
		assert.ok(Object.isFrozen(resolution));
	});

	test('the spawn module links no fs: its fs imports are types, and the registry imports nothing', () => {
		const imports_of = (file: string): Array<{ specifier: string; type_only: boolean }> => {
			const source = ts.createSourceFile(
				file,
				readFileSync(join(paths.lib, file), 'utf8'),
				ts.ScriptTarget.Latest
			);
			const found: Array<{ specifier: string; type_only: boolean }> = [];
			for (const statement of source.statements) {
				if (ts.isImportDeclaration(statement) || ts.isExportDeclaration(statement)) {
					const specifier = statement.moduleSpecifier;
					if (specifier && ts.isStringLiteral(specifier)) {
						const type_only = ts.isImportDeclaration(statement)
							? statement.importClause?.phaseModifier === ts.SyntaxKind.TypeKeyword
							: statement.isTypeOnly;
						found.push({ specifier: specifier.text, type_only });
					}
				}
			}
			return found;
		};
		const spawn_fs_imports = imports_of('spawn.ts').filter((i) => i.specifier === './fs.ts');
		assert.isNotEmpty(spawn_fs_imports);
		for (const i of spawn_fs_imports)
			assert.ok(i.type_only, 'spawn.ts imports fs.ts for types only');
		assert.deepEqual(imports_of('internal/fs_handles.ts'), []);
	});
});
