import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import {
	chmodSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { join, relative } from 'node:path';

import { spawn, type Spawn, type SpawnProgram } from '#lib/spawn.ts';
import { SpawnError, is_spawn_error, type SpawnErrorKind } from '#lib/spawn_error.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

const SYSTEM_PATH = process.env.PATH;

let sandbox: FsSandbox;
let bin: string;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	bin = join(sandbox.base, 'bin');
	mkdirSync(bin);
});

afterEach(() => {
	process.env.PATH = SYSTEM_PATH;
	remove_fs_sandbox(sandbox);
});

const write_script = (path: string, body: string, mode = 0o755): void => {
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

const assert_rejects = async (fn: () => Promise<unknown>): Promise<void> => {
	try {
		await fn();
	} catch (error) {
		assert.instanceOf(error, TypeError);
		return;
	}
	assert.fail('expected a rejection');
};

const assert_throws_type = (fn: () => unknown): void => {
	assert.throws(fn, TypeError);
};

const env_program = (s: Spawn = spawn, options?: Parameters<Spawn['program']>[1]): SpawnProgram =>
	s.program('env', options);

const read_env = async (program: SpawnProgram): Promise<Map<string, string>> => {
	const output = await program.out([]);
	assert.strictEqual(output.kind, 'exited');
	const vars = new Map<string, string>();
	for (const line of output.stdout.split('\n')) {
		if (!line) continue;
		const i = line.indexOf('=');
		vars.set(line.slice(0, i), line.slice(i + 1));
	}
	return vars;
};

describe('SpawnError', () => {
	test('is typed and frozen, and matches across copies of the class', () => {
		const error = new SpawnError('not_found', 'git');
		assert.strictEqual(error.kind, 'not_found');
		assert.strictEqual(error.program, 'git');
		assert.strictEqual(error.name, 'SpawnError');
		assert.ok(Object.isFrozen(SpawnError));
		assert.ok(Object.isFrozen(SpawnError.prototype));
		assert.ok(is_spawn_error(error, 'not_found'));
		assert.ok(!is_spawn_error(error, 'not_granted'));

		class Other extends Error {
			kind = 'launch_failed';
		}
		Object.defineProperty(Other, 'name', { value: 'SpawnError' });
		assert.ok(is_spawn_error(new Other(), 'launch_failed'));
		assert.ok(!is_spawn_error(new Error('x')));
		assert.ok(!is_spawn_error('SpawnError'));
	});
});

describe('resolution', () => {
	test('a bare name walks PATH to the first regular executable', () => {
		write_script(join(bin, 'tool'), 'exit 0');
		mkdirSync(join(sandbox.root, 'tool'));
		write_script(join(sandbox.outside, 'tool'), 'exit 9');
		// the directory is skipped; the first regular executable wins over a later one
		process.env.PATH = [sandbox.root, bin, sandbox.outside].join(':');
		const tool = spawn.program('tool');
		assert.strictEqual(tool.name, 'tool');
		assert.strictEqual(tool.path, realpathSync(join(bin, 'tool')));
	});

	test('a relative or empty PATH entry never reaches the working directory', () => {
		process.env.PATH = ['', '.', 'relative/dir', bin].join(':');
		write_script(join(bin, 'on_path'), 'exit 0');
		// the working directory holds it, without execute permission, so a walk there would throw
		assert.isNull(spawn.find('package.json'));
		assert.ok(spawn.find('on_path'));
	});

	test('`program` throws `not_found` and `find` returns null on the miss', () => {
		process.env.PATH = bin;
		assert_spawn_error(() => spawn.program('nope'), 'not_found');
		assert.isNull(spawn.find('nope'));
		delete process.env.PATH;
		assert.isNull(spawn.find('nope'));
	});

	test('a regular file without execute permission is `not_executable` when no other is found', () => {
		write_script(join(bin, 'plain'), 'exit 0', 0o644);
		process.env.PATH = bin;
		assert_spawn_error(() => spawn.program('plain'), 'not_executable');
		assert_spawn_error(() => spawn.find('plain'), 'not_executable');
		write_script(join(sandbox.root, 'plain'), 'exit 0');
		process.env.PATH = [bin, sandbox.root].join(':');
		assert.ok(spawn.find('plain'));
	});

	test('`.bat` and `.cmd` are refused by name and by real path', () => {
		write_script(join(bin, 'script.cmd'), 'exit 0');
		symlinkSync(join(bin, 'script.cmd'), join(bin, 'clean_name'));
		process.env.PATH = bin;
		assert_spawn_error(() => spawn.program('script.cmd'), 'not_executable');
		assert_spawn_error(() => spawn.program('thing.BAT'), 'not_executable');
		assert_spawn_error(() => spawn.program('thing.bat.'), 'not_executable');
		assert_spawn_error(() => spawn.program('clean_name'), 'not_executable');
	});

	test('the realpath is pinned, surviving a later change to PATH and the symlink', async () => {
		write_script(join(bin, 'real_a'), 'echo a');
		write_script(join(bin, 'real_b'), 'echo b');
		symlinkSync(join(bin, 'real_a'), join(bin, 'tool'));
		process.env.PATH = bin;
		const tool = spawn.program('tool');
		assert.strictEqual(tool.path, join(bin, 'real_a'));
		rmSync(join(bin, 'tool'));
		symlinkSync(join(bin, 'real_b'), join(bin, 'tool'));
		process.env.PATH = '';
		const output = await tool.out([]);
		assert.strictEqual(output.kind, 'exited');
		assert.strictEqual(output.stdout, 'a\n');
	});

	test('a name with / resolves against the project root', () => {
		const tsc = spawn.find('node_modules/.bin/tsc');
		assert.ok(tsc);
		assert.strictEqual(tsc.name, 'node_modules/.bin/tsc');
		assert.strictEqual(tsc.path, realpathSync('node_modules/.bin/tsc'));
		assert.isNull(spawn.find('node_modules/.bin/absent_program'));
		assert.isNull(spawn.find('node_modules/.bin/tsc/'));
	});

	test('an absolute name or one climbing above the project is `not_granted`', () => {
		for (const name of [
			'/bin/sh',
			'../sibling',
			'a/../../sibling',
			'./../x',
			'node_modules/../../x'
		]) {
			assert_spawn_error(() => spawn.program(name), 'not_granted');
			assert_spawn_error(() => spawn.find(name), 'not_granted');
		}
		// a `..` that stays beneath the project isn't a climb, only a miss
		assert.isNull(spawn.find('node_modules/../not_a_real_program_name'));
	});

	test('a malformed name is a TypeError from every entry point', () => {
		const names: Array<any> = ['', 'a\0b', 'a\ud800b', undefined, null, 1, {}];
		for (const name of names) {
			assert_throws_type(() => spawn.program(name));
			assert_throws_type(() => spawn.find(name));
			assert_throws_type(() => spawn.narrow([name]));
		}
		assert_throws_type(() => spawn.narrow('git' as any));
	});

	test('options are read and checked before the name resolves', () => {
		process.env.PATH = bin;
		assert_throws_type(() => spawn.find('missing', { pass_env: 1 } as any));
		assert_throws_type(() => spawn.program('missing', { env: 1 } as any));
		assert_throws_type(() => spawn.find('missing', [] as any));
		assert_throws_type(() => spawn.program('missing', [] as any));
		let reads = 0;
		const options: any = {
			get pass_env() {
				reads++;
				return [];
			}
		};
		assert.isNull(spawn.find('missing', options));
		assert.strictEqual(reads, 1);
	});
});

describe('names containing /', () => {
	let dir: string;
	let abs: string;

	beforeEach(() => {
		const parent = join(process.cwd(), '.gro');
		mkdirSync(parent, { recursive: true });
		abs = mkdtempSync(join(parent, 'spawn-test-'));
		dir = relative(process.cwd(), abs);
	});

	afterEach(() => {
		rmSync(abs, { recursive: true, force: true });
	});

	test('pass the lexical project-absolute path as argv0, `name` staying as granted', async () => {
		symlinkSync('/bin/sh', join(abs, 'granted_sh'));
		const name = `./${dir}/skipped/../granted_sh`;
		const program = spawn.program(name);
		assert.strictEqual(program.name, name);
		const args = ['-c', 'printf %s "$0"'];
		assert.strictEqual((await program.out(args)).stdout, join(abs, 'granted_sh'));
		const elsewhere = await program.out(args, { cwd: sandbox.dir });
		assert.strictEqual(elsewhere.stdout, join(abs, 'granted_sh'));
	});

	test('a file reached with a trailing `/` or `/.` never pins', () => {
		write_script(join(abs, 'ok'), 'exit 0');
		assert.ok(spawn.find(`${dir}/ok`));
		for (const name of [`${dir}/ok/`, `${dir}/ok/.`, `${dir}/ok/./`, `${dir}/ok/./.`]) {
			assert.isNull(spawn.find(name), name);
			assert_spawn_error(() => spawn.program(name), 'not_found');
		}
		assert.isNull(spawn.find(`${dir}/ok/x`));
		// a `..` resolves lexically, before any file is looked at
		assert.ok(spawn.find(`${dir}/ok/../ok`));
	});

	test('a file with no `#!` is refused here too', () => {
		writeFileSync(join(abs, 'plain'), 'echo hi\n');
		chmodSync(join(abs, 'plain'), 0o755);
		assert_spawn_error(() => spawn.program(`${dir}/plain`), 'not_executable');
	});
});

describe('file formats at the pin', () => {
	const write_bytes = (name: string, bytes: Uint8Array | string): void => {
		writeFileSync(join(bin, name), bytes);
		chmodSync(join(bin, name), 0o755);
	};

	test('a file that is neither `#!` nor ELF is `not_executable`', () => {
		process.env.PATH = bin;
		write_bytes('no_shebang', 'echo hi\n');
		write_bytes('mz', new Uint8Array([0x4d, 0x5a, 0x90, 0x00, 0x03]));
		write_bytes('empty', '');
		write_bytes('one_byte', '#');
		write_bytes('elf_no_marker', 'ELF!\n');
		for (const name of ['no_shebang', 'mz', 'empty', 'one_byte', 'elf_no_marker']) {
			assert_spawn_error(() => spawn.program(name), 'not_executable');
			assert_spawn_error(() => spawn.find(name), 'not_executable');
		}
	});

	test('a near miss of either magic is refused', () => {
		process.env.PATH = bin;
		const near_misses = [
			[0x00, 0x21, 0x0a],
			[0x23, 0x00, 0x0a],
			[0x00, 0x45, 0x4c, 0x46],
			[0x7f, 0x00, 0x4c, 0x46],
			[0x7f, 0x45, 0x00, 0x46],
			[0x7f, 0x45, 0x4c, 0x00]
		];
		near_misses.forEach((bytes, i) => {
			write_bytes(`near_${i}`, new Uint8Array(bytes));
			assert_spawn_error(() => spawn.program(`near_${i}`), 'not_executable');
		});
	});

	test('a link is inspected at its real path', () => {
		process.env.PATH = bin;
		write_bytes('no_shebang', 'echo hi\n');
		symlinkSync(join(bin, 'no_shebang'), join(bin, 'link'));
		const error = assert_spawn_error(() => spawn.program('link'), 'not_executable');
		assert.include(error.message, ` at ${realpathSync(join(bin, 'no_shebang'))} starts`);
	});

	test('a shebang script and a real ELF pin', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'script'), 'exit 0');
		symlinkSync(realpathSync(process.execPath), join(bin, 'real_elf'));
		assert.ok(spawn.program('script'));
		assert.strictEqual(spawn.program('real_elf').path, realpathSync(process.execPath));
	});

	// the module reads the platform when it loads, so a fresh copy loads under a stubbed one
	const load_spawn_on = async (platform: NodeJS.Platform): Promise<Spawn> => {
		const descriptor = Object.getOwnPropertyDescriptor(process, 'platform')!;
		Object.defineProperty(process, 'platform', { ...descriptor, value: platform });
		vi.resetModules();
		try {
			return (await import('#lib/spawn.ts')).spawn;
		} finally {
			Object.defineProperty(process, 'platform', descriptor);
			vi.resetModules();
		}
	};

	const ELF = [0x7f, 0x45, 0x4c, 0x46];
	const MACH_O_MAGICS = [
		[0xfe, 0xed, 0xfa, 0xce],
		[0xfe, 0xed, 0xfa, 0xcf],
		[0xce, 0xfa, 0xed, 0xfe],
		[0xcf, 0xfa, 0xed, 0xfe],
		[0xca, 0xfe, 0xba, 0xbe]
	];

	test('only the first bytes are read, so a file with the ELF magic pins', async () => {
		process.env.PATH = bin;
		write_bytes('fake_elf', new Uint8Array([...ELF, 0x00]));
		assert.ok((await load_spawn_on('linux')).program('fake_elf'));
	});

	test('off macOS the Mach-O magics are refused', async () => {
		process.env.PATH = bin;
		const on_linux = await load_spawn_on('linux');
		MACH_O_MAGICS.forEach((bytes, i) => {
			write_bytes(`mach_o_${i}`, new Uint8Array(bytes));
			assert_spawn_error(() => on_linux.program(`mach_o_${i}`), 'not_executable');
		});
	});

	test('on macOS the Mach-O magics and `#!` pin, and ELF and near misses are refused', async () => {
		process.env.PATH = bin;
		const on_macos = await load_spawn_on('darwin');
		MACH_O_MAGICS.forEach((bytes, i) => {
			write_bytes(`mach_o_${i}`, new Uint8Array(bytes));
			assert.ok(on_macos.program(`mach_o_${i}`));
		});
		write_script(join(bin, 'script'), 'exit 0');
		assert.ok(on_macos.program('script'));
		const refused = [
			ELF,
			[0xfe, 0xed, 0xfa, 0xcd],
			[0x00, 0xed, 0xfa, 0xce],
			[0xca, 0xfe, 0xba, 0xbf],
			[0xfe, 0xed, 0xfa]
		];
		refused.forEach((bytes, i) => {
			write_bytes(`refused_${i}`, new Uint8Array(bytes));
			assert_spawn_error(() => on_macos.program(`refused_${i}`), 'not_executable');
		});
	});

	test.skipIf(process.getuid?.() === 0)(
		'an executable this user cannot read passes, as it cannot be inspected',
		() => {
			process.env.PATH = bin;
			write_script(join(bin, 'unreadable'), 'exit 0', 0o111);
			assert.ok(spawn.program('unreadable'));
		}
	);

	test('a real path that is not UTF-8 is refused', () => {
		process.env.PATH = bin;
		const bad_dir = Buffer.concat([Buffer.from(`${bin}/bad_`), Buffer.from([0xff])]);
		mkdirSync(bad_dir);
		const real = Buffer.concat([bad_dir, Buffer.from('/real')]);
		writeFileSync(real, '#!/bin/sh\nexit 0\n');
		chmodSync(real, 0o755);
		symlinkSync(real, join(bin, 'tool'));
		const error = assert_spawn_error(() => spawn.program('tool'), 'not_executable');
		// the fatal decoder's own refusal, not a lossy name that then fails to open
		assert.instanceOf(error.cause, TypeError);
	});
});

describe('narrow', () => {
	test('restricts program and find to the named names, and can only shrink', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'exit 0');
		write_script(join(bin, 'b'), 'exit 0');
		const a_only = spawn.narrow(['a']);
		assert.ok(a_only.program('a'));
		assert.ok(a_only.find('a'));
		assert_spawn_error(() => a_only.program('b'), 'not_granted');
		assert_spawn_error(() => a_only.find('b'), 'not_granted');
		// a narrower root names only what it allows
		assert.ok(a_only.narrow(['a']).program('a'));
		assert_spawn_error(() => a_only.narrow(['b']), 'not_granted');
		assert_spawn_error(() => spawn.narrow(['a', 'b']).narrow(['c']), 'not_granted');
		const empty = spawn.narrow([]);
		assert_spawn_error(() => empty.program('a'), 'not_granted');
	});

	test('a name not on the set is refused before any resolution, spelled exactly', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'exit 0');
		const only_a = spawn.narrow(['a']);
		assert_spawn_error(() => only_a.program('./a'), 'not_granted');
		assert_spawn_error(() => only_a.program('../escape'), 'not_granted');
		assert_spawn_error(() => only_a.program('ghost'), 'not_granted');
	});

	test('a narrowed root refuses `env`, `pass_env: all`, and globs, down any chain', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'exit 0');
		const once = spawn.narrow(['a']);
		const twice = once.narrow(['a']).narrow(['a']);
		const refused: Array<any> = [
			{ env: { LD_PRELOAD: '/tmp/evil.so' } },
			{ env: {} },
			{ pass_env: 'all' },
			{ pass_env: ['HOME'], env: { X: '1' } },
			{ pass_env: ['GIT_*'] },
			{ pass_env: ['HOME', 'A*'] },
			{ pass_env: 'all', omit_env: ['A'] }
		];
		for (const narrowed of [once, twice]) {
			for (const options of refused) {
				assert_spawn_error(() => narrowed.program('a', options), 'not_granted');
				assert_spawn_error(() => narrowed.find('a', options), 'not_granted');
			}
		}
		// a lone `*` is malformed anywhere, before any grant is checked
		assert_throws_type(() => twice.program('a', { pass_env: ['*'] }));
		// `omit_env` without `'all'` is malformed anywhere too
		assert_throws_type(() => twice.program('a', { omit_env: ['A'] }));
		// the root keeps all three, and a narrowed root keeps named `pass_env`
		assert.ok(spawn.program('a', { env: { X: '1' }, pass_env: 'all' }));
		assert.ok(spawn.program('a', { pass_env: ['GIT_*'] }));
		assert.ok(twice.program('a', { pass_env: ['HOME', 'GIT_DIR'] }));
		assert.ok(twice.program('a'));
	});

	test('a narrowed root takes `env: undefined` and reads `pass_env` once', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'exit 0');
		const narrowed = spawn.narrow(['a']);
		assert.ok(narrowed.program('a', { env: undefined }));
		let reads = 0;
		const options: any = {
			get pass_env() {
				reads++;
				return reads === 1 ? ['HOME'] : 'all';
			}
		};
		assert.ok(narrowed.program('a', options));
		assert.strictEqual(reads, 1);
	});

	test('a program built at the root keeps its env, the way to hand a library one', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'printf %s "$X"');
		const program = spawn.program('a', { env: { X: 'set' } });
		const output = await program.out([]);
		assert.strictEqual(output.stdout, 'set');
	});

	test('the names array is read once and copied', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'a'), 'exit 0');
		write_script(join(bin, 'b'), 'exit 0');
		let reads = 0;
		const names: Array<string> = [];
		Object.defineProperty(names, '0', {
			get() {
				reads++;
				return reads === 1 ? 'a' : 'b';
			},
			enumerable: true
		});
		names.length = 1;
		const narrowed = spawn.narrow(names);
		assert.strictEqual(reads, 1);
		assert.ok(narrowed.find('a'));
		assert_spawn_error(() => narrowed.find('b'), 'not_granted');
	});
});

describe('calls', () => {
	test('`run` resolves a non-zero exit as a result', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'fails'), 'exit 3');
		write_script(join(bin, 'ok'), 'exit 0');
		assert.deepEqual({ ...(await spawn.program('ok').run([])) }, { kind: 'exited', code: 0 });
		assert.deepEqual({ ...(await spawn.program('fails').run([])) }, { kind: 'exited', code: 3 });
	});

	test('a signal is a `signaled` result', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'selfkill'), 'kill -TERM $$');
		const result = await spawn.program('selfkill').run([]);
		assert.deepEqual({ ...result }, { kind: 'signaled', signal: 'SIGTERM' });
	});

	test('`out` captures stdout and stderr with no shell', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'both'), 'printf "out %s" "$1"; printf "err" >&2; exit 2');
		const output = await spawn.program('both').out(['a b;c $(x)']);
		assert.strictEqual(output.kind, 'exited');
		assert.strictEqual(output.kind === 'exited' ? output.code : -1, 2);
		assert.strictEqual(output.stdout, 'out a b;c $(x)');
		assert.strictEqual(output.stderr, 'err');
		assert.strictEqual(output.truncated, false);
	});

	test('a launch failure throws `launch_failed`, and a bad arg throws TypeError', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'vanishing'), 'exit 0');
		const program = spawn.program('vanishing');
		rmSync(join(bin, 'vanishing'));
		for (const call of [() => program.run([]), () => program.out([])]) {
			try {
				await call();
				assert.fail('expected a launch failure');
			} catch (error) {
				assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
				assert.ok(error.cause);
				assert.include(error.message, 'vanishing');
			}
		}
	});

	test('args allow an empty string', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'count'), 'printf "%s[%s]" "$#" "$1"');
		const output = await spawn.program('count').out(['']);
		assert.strictEqual(output.stdout, '1[]');
	});

	test('args are read once, element by element', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'first'), 'printf %s "$1"');
		let reads = 0;
		const args: Array<string> = [];
		Object.defineProperty(args, '0', {
			get() {
				reads++;
				return reads === 1 ? 'first_read' : 'second_read';
			},
			enumerable: true
		});
		args.length = 1;
		const output = await spawn.program('first').out(args);
		assert.strictEqual(reads, 1);
		assert.strictEqual(output.stdout, 'first_read');
	});

	test('args refuse non-strings, NUL, and lone surrogates', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		const bad_args: Array<any> = ['a\0b', 'a\ud800b', 1, null, undefined];
		for (const bad of bad_args) {
			await assert_rejects(() => program.run([bad]));
			await assert_rejects(() => program.out(['x', bad]));
		}
		await assert_rejects(() => program.run('git' as any));
		await assert_rejects(() => program.run([], [] as any));
		await assert_rejects(() => program.out([], [] as any));
	});

	test('exec passes the granted name as argv0, not the real path', async () => {
		symlinkSync('/bin/sh', join(bin, 'granted_sh'));
		process.env.PATH = bin;
		const program = spawn.program('granted_sh');
		assert.strictEqual(program.path, realpathSync('/bin/sh'));
		const output = await program.out(['-c', 'printf %s "$0"']);
		assert.strictEqual(output.kind, 'exited');
		assert.strictEqual(output.stdout, 'granted_sh');
	});

	test('`out` gives the child `/dev/null` for stdin', async () => {
		const output = await spawn.program('readlink').out(['/proc/self/fd/0']);
		assert.strictEqual(output.stdout, '/dev/null\n');
	});

	test('the cap destroys the streams, so a grandchild holding them open cannot hold the call', async () => {
		process.env.PATH = [bin, SYSTEM_PATH].join(':');
		const pid_path = join(bin, 'grandchild_pid');
		// the backgrounded `sleep` inherits stdout and outlives the killed script
		write_script(join(bin, 'hold'), `sleep 10 &\necho $! > '${pid_path}'\nprintf 123456\nwait`);
		try {
			const output = await spawn.program('hold').out([], { max_bytes: 5 });
			assert.strictEqual(output.truncated, true);
			assert.strictEqual(output.stdout, '12345');
		} finally {
			try {
				process.kill(Number(readFileSync(pid_path, 'utf8').trim()), 'SIGKILL');
			} catch {
				// the grandchild already ended
			}
		}
	});

	test('`out` is capped per stream: over the cap the child is killed and the result is `truncated`', async () => {
		process.env.PATH = bin;
		// loops forever until killed, so only the cap ends this test
		write_script(join(bin, 'spew'), 'while :; do printf abcdefghij; done');
		const output = await spawn.program('spew').out([], { max_bytes: 2048 });
		assert.strictEqual(output.truncated, true);
		assert.strictEqual(output.stdout.length, 2048);
		assert.strictEqual(output.kind, 'signaled');
		assert.strictEqual(output.kind === 'signaled' ? output.signal : '', 'SIGKILL');
	});

	test('a stream exactly at the cap is not truncated', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'five'), 'printf 12345');
		const output = await spawn.program('five').out([], { max_bytes: 5 });
		assert.strictEqual(output.stdout, '12345');
		assert.strictEqual(output.truncated, false);
		const none = await spawn.program('five').out([], { max_bytes: 0 });
		assert.strictEqual(none.stdout, '');
		assert.strictEqual(none.truncated, true);
	});

	test('a bad `max_bytes` is a TypeError', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		for (const max_bytes of [-1, 1.5, NaN, Number.MAX_SAFE_INTEGER + 1, '1' as any, null as any]) {
			await assert_rejects(() => program.out([], { max_bytes }));
		}
	});

	test('`cwd` is a handle: default project root, real handle honored, string refused', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'pwd_it'), 'pwd');
		const program = spawn.program('pwd_it');
		assert.strictEqual((await program.out([])).stdout.trim(), process.cwd());
		const handled = await program.out([], { cwd: sandbox.dir });
		assert.strictEqual(handled.stdout.trim(), sandbox.root);
		const bad_cwds: Array<any> = ['/tmp', 1, null, { path: 1 }, { path: 'relative/dir' }];
		for (const cwd of bad_cwds) {
			await assert_rejects(() => program.run([], { cwd }));
			await assert_rejects(() => program.out([], { cwd }));
		}
	});

	test('a missing cwd is `launch_failed`', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		const gone = { path: join(sandbox.root, 'no_such_dir') } as any;
		try {
			await program.out([], { cwd: gone });
			assert.fail('expected a launch failure');
		} catch (error) {
			assert.ok(is_spawn_error(error, 'launch_failed'), String(error));
		}
	});

	test("`out` options read each option once, so a getter can't switch", async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'pwd_it'), 'pwd');
		const program = spawn.program('pwd_it');
		let reads = 0;
		const options: any = {
			get cwd() {
				reads++;
				return reads === 1 ? sandbox.dir : { path: sandbox.outside };
			}
		};
		const output = await program.out([], options);
		assert.strictEqual(reads, 1);
		assert.strictEqual(output.stdout.trim(), sandbox.root);
	});

	test('`run` options read cwd once too', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		let reads = 0;
		const options: any = {
			get cwd() {
				reads++;
				return sandbox.dir;
			}
		};
		await spawn.program('ok').run([], options);
		assert.strictEqual(reads, 1);
	});

	test('a per-call env option has no effect', async () => {
		process.env.PATH = bin;
		const dir = join(bin, 'env_bin');
		mkdirSync(dir);
		symlinkSync(realpathSync('/usr/bin/env'), join(dir, 'env'));
		process.env.PATH = dir;
		const output = await env_program().out([], { env: { PER_CALL: '1' } } as any);
		assert.notInclude(output.stdout, 'PER_CALL');
	});
});

describe('the child environment', () => {
	let keep: Record<string, string | undefined>;

	beforeEach(() => {
		keep = { NO_COLOR: process.env.NO_COLOR, FORCE_COLOR: process.env.FORCE_COLOR };
		delete process.env.NO_COLOR;
		delete process.env.FORCE_COLOR;
		process.env.GRO_SPAWN_TEST_SECRET = 'leak';
		process.env.GRO_SPAWN_TEST_GIT_A = '1';
		process.env.GRO_SPAWN_TEST_GIT_B = '2';
		const env_bin = join(bin, 'env_bin');
		mkdirSync(env_bin);
		symlinkSync(realpathSync('/usr/bin/env'), join(env_bin, 'env'));
		process.env.PATH = env_bin;
	});

	afterEach(() => {
		delete process.env.GRO_SPAWN_TEST_SECRET;
		delete process.env.GRO_SPAWN_TEST_GIT_A;
		delete process.env.GRO_SPAWN_TEST_GIT_B;
		for (const name of ['NO_COLOR', 'FORCE_COLOR']) {
			const value = keep[name];
			if (value === undefined) delete process.env[name];
			else process.env[name] = value;
		}
	});

	test('is cleared to the base set and pins LC_ALL', async () => {
		const vars = await read_env(env_program());
		assert.strictEqual(vars.get('LC_ALL'), 'C');
		assert.strictEqual(vars.get('PATH'), process.env.PATH);
		assert.isFalse(vars.has('GRO_SPAWN_TEST_SECRET'));
		assert.isFalse(vars.has('NO_COLOR'));
		assert.isFalse(vars.has('FORCE_COLOR'));
	});

	test('the host color settings pass when present', async () => {
		process.env.NO_COLOR = '1';
		process.env.FORCE_COLOR = '3';
		const vars = await read_env(env_program());
		assert.strictEqual(vars.get('NO_COLOR'), '1');
		assert.strictEqual(vars.get('FORCE_COLOR'), '3');
	});

	test("`omit_env` leaves exact names out of `pass_env: 'all'`, the rest forwarded", async () => {
		const vars = await read_env(
			env_program(spawn, { pass_env: 'all', omit_env: ['GRO_SPAWN_TEST_SECRET', 'NOT_SET'] })
		);
		assert.isFalse(vars.has('GRO_SPAWN_TEST_SECRET'));
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_GIT_A'), '1');
	});

	test('`omit_env` is case-sensitive', async () => {
		const vars = await read_env(
			env_program(spawn, { pass_env: 'all', omit_env: ['gro_spawn_test_secret'] })
		);
		assert.isTrue(vars.has('GRO_SPAWN_TEST_SECRET'));
	});

	test('`omit_env` changes only what is forwarded, so `env` still sets a name', async () => {
		const vars = await read_env(
			env_program(spawn, {
				pass_env: 'all',
				omit_env: ['GRO_SPAWN_TEST_SECRET'],
				env: { GRO_SPAWN_TEST_SECRET: 'explicit' }
			})
		);
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_SECRET'), 'explicit');
	});

	test('`omit_env` is read once, its elements too, and a later change to it does nothing', async () => {
		let reads = 0;
		let element_reads = 0;
		const omit_env: Array<string> = [];
		Object.defineProperty(omit_env, '0', {
			get() {
				element_reads++;
				return element_reads === 1 ? 'GRO_SPAWN_TEST_SECRET' : 'GRO_SPAWN_TEST_GIT_A';
			},
			enumerable: true
		});
		omit_env.length = 1;
		const options: any = {
			pass_env: 'all',
			get omit_env() {
				reads++;
				return omit_env;
			}
		};
		const vars = await read_env(env_program(spawn, options));
		assert.strictEqual(reads, 1);
		assert.strictEqual(element_reads, 1);
		assert.isFalse(vars.has('GRO_SPAWN_TEST_SECRET'));
		assert.isTrue(vars.has('GRO_SPAWN_TEST_GIT_A'));
	});

	test('`omit_env` names survive a host variable named after a prototype member', async () => {
		process.env.__proto__x = 'kept';
		try {
			const vars = await read_env(
				env_program(spawn, { pass_env: 'all', omit_env: ['__proto__', 'constructor'] })
			);
			assert.isTrue(vars.has('__proto__x'));
			assert.isTrue(vars.has('GRO_SPAWN_TEST_SECRET'));
		} finally {
			delete process.env.__proto__x;
		}
	});

	test('`pass_env` adds names and `PREFIX_*` globs', async () => {
		const vars = await read_env(
			env_program(spawn, { pass_env: ['GRO_SPAWN_TEST_SECRET', 'GRO_SPAWN_TEST_GIT_*', 'NOT_SET'] })
		);
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_SECRET'), 'leak');
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_GIT_A'), '1');
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_GIT_B'), '2');
		assert.isFalse(vars.has('NOT_SET'));
	});

	test('a `PREFIX_*` glob passes only its prefix', async () => {
		const vars = await read_env(env_program(spawn, { pass_env: ['GRO_SPAWN_TEST_GIT_*'] }));
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_GIT_A'), '1');
		assert.isFalse(vars.has('GRO_SPAWN_TEST_SECRET'));
	});

	test('`pass_env` elements are read once', async () => {
		let reads = 0;
		const pass_env: Array<string> = [];
		Object.defineProperty(pass_env, '0', {
			get() {
				reads++;
				return reads === 1 ? 'GRO_SPAWN_TEST_GIT_A' : 'GRO_SPAWN_TEST_SECRET';
			},
			enumerable: true
		});
		pass_env.length = 1;
		const vars = await read_env(env_program(spawn, { pass_env }));
		assert.strictEqual(reads, 1);
		assert.strictEqual(vars.get('GRO_SPAWN_TEST_GIT_A'), '1');
		assert.isFalse(vars.has('GRO_SPAWN_TEST_SECRET'));
	});

	test("a call's env is a copy, so what Node writes into it doesn't stay on the handle", async () => {
		const program = env_program();
		const keep_coverage = process.env.NODE_V8_COVERAGE;
		process.env.NODE_V8_COVERAGE = 'gro_spawn_test_coverage';
		try {
			assert.strictEqual(
				(await read_env(program)).get('NODE_V8_COVERAGE'),
				'gro_spawn_test_coverage'
			);
		} finally {
			if (keep_coverage === undefined) delete process.env.NODE_V8_COVERAGE;
			else process.env.NODE_V8_COVERAGE = keep_coverage;
		}
		if (keep_coverage === undefined) {
			assert.isFalse((await read_env(program)).has('NODE_V8_COVERAGE'));
		}
	});

	test("`pass_env: 'all'` is the host's whole environment, explicit values over it", async () => {
		process.env.GRO_SPAWN_TEST_OVERRIDE = 'host';
		try {
			const vars = await read_env(
				env_program(spawn, { pass_env: 'all', env: { GRO_SPAWN_TEST_OVERRIDE: 'explicit' } })
			);
			assert.strictEqual(vars.get('GRO_SPAWN_TEST_SECRET'), 'leak');
			assert.strictEqual(vars.get('GRO_SPAWN_TEST_OVERRIDE'), 'explicit');
			assert.strictEqual(vars.get('LC_ALL'), process.env.LC_ALL);
		} finally {
			delete process.env.GRO_SPAWN_TEST_OVERRIDE;
		}
	});

	test('explicit values win, and an empty value is a value', async () => {
		const vars = await read_env(env_program(spawn, { env: { LC_ALL: 'POSIX', EMPTY: '' } }));
		assert.strictEqual(vars.get('LC_ALL'), 'POSIX');
		assert.strictEqual(vars.get('EMPTY'), '');
	});

	test('the env is fixed when the handle is made', async () => {
		const program = env_program(spawn, { pass_env: ['GRO_SPAWN_TEST_SECRET'] });
		process.env.GRO_SPAWN_TEST_SECRET = 'changed';
		assert.strictEqual((await read_env(program)).get('GRO_SPAWN_TEST_SECRET'), 'leak');
	});

	test('the child env object is null-prototype, so a polluted name never reaches a child', async () => {
		(Object.prototype as any).GRO_SPAWN_INJ = 'y';
		try {
			const vars = await read_env(env_program());
			assert.isFalse(vars.has('GRO_SPAWN_INJ'));
			const passed = await read_env(env_program(spawn, { pass_env: ['GRO_SPAWN_INJ'] }));
			assert.isFalse(passed.has('GRO_SPAWN_INJ'));
		} finally {
			delete (Object.prototype as any).GRO_SPAWN_INJ;
		}
	});

	test('a polluted spawn option reaches no call', async () => {
		// an option `launch` leaves unset, refused if it reached Node, whose own copy of its
		// options into a null-prototype object is what keeps it out on the Nodes Gro supports
		(Object.prototype as any).uid = 'not_a_uid';
		try {
			const output = await env_program().out([]);
			assert.strictEqual(output.kind, 'exited');
		} finally {
			delete (Object.prototype as any).uid;
		}
	});

	test('options are own properties, so a polluted `Object.prototype` sets none', async () => {
		const polluted = Object.prototype as any;
		polluted.pass_env = 'all';
		let root_program: SpawnProgram;
		try {
			root_program = env_program(spawn, {});
		} finally {
			delete polluted.pass_env;
		}
		assert.isFalse((await read_env(root_program)).has('GRO_SPAWN_TEST_SECRET'));

		polluted.omit_env = ['GRO_SPAWN_TEST_SECRET'];
		let omit_program: SpawnProgram;
		try {
			// no own `pass_env`, so a polluted `omit_env` alone would be a `TypeError`
			env_program(spawn, {});
			omit_program = env_program(spawn, { pass_env: 'all' });
		} finally {
			delete polluted.omit_env;
		}
		assert.isTrue((await read_env(omit_program)).has('GRO_SPAWN_TEST_SECRET'));

		polluted.env = { X: '1' };
		try {
			assert.ok(spawn.narrow(['env']).program('env', {}));
		} finally {
			delete polluted.env;
		}

		// a quiet script rather than `env`, since `run` inherits stdio
		write_script(join(process.env.PATH!, 'quiet'), 'exit 0');
		const program = spawn.program('quiet');
		polluted.cwd = 'not_a_handle';
		polluted.max_bytes = 'not_a_number';
		let calls: Array<Promise<unknown>>;
		try {
			calls = [program.out([], {}), program.run([], {})];
		} finally {
			delete polluted.cwd;
			delete polluted.max_bytes;
		}
		await Promise.all(calls);
	});

	test('`pass_env` reads host variables as own properties only', async () => {
		const vars = await read_env(env_program(spawn, { pass_env: ['toString', 'hasOwnProperty'] }));
		assert.isFalse(vars.has('toString'));
		assert.isFalse(vars.has('hasOwnProperty'));
	});

	test("options read each value once, so a getter or a growing record can't switch", async () => {
		let pass_reads = 0;
		let env_reads = 0;
		const options: any = {
			get pass_env() {
				pass_reads++;
				return ['GRO_SPAWN_TEST_SECRET'];
			},
			env: {
				get VALUE() {
					env_reads++;
					return env_reads === 1 ? 'first' : 'second';
				}
			}
		};
		const vars = await read_env(env_program(spawn, options));
		assert.strictEqual(pass_reads, 1);
		assert.strictEqual(env_reads, 1);
		assert.strictEqual(vars.get('VALUE'), 'first');
	});

	test('malformed env options are a TypeError', () => {
		const bad: Array<any> = [
			1,
			null,
			{ pass_env: 'x' },
			{ pass_env: [1] },
			{ pass_env: ['A=B'] },
			{ pass_env: [''] },
			{ pass_env: ['*'] },
			{ pass_env: ['A*B'] },
			{ omit_env: ['A'] },
			{ omit_env: [] },
			{ pass_env: ['HOME'], omit_env: ['A'] },
			{ pass_env: 'all', omit_env: 'A' },
			{ pass_env: 'all', omit_env: { 0: 'A', length: 1 } },
			{ pass_env: 'all', omit_env: [1] },
			{ pass_env: 'all', omit_env: [''] },
			{ pass_env: 'all', omit_env: ['A=B'] },
			{ pass_env: 'all', omit_env: ['A*'] },
			{ pass_env: 'all', omit_env: ['*'] },
			{ pass_env: 'all', omit_env: ['A\0B'] },
			{ pass_env: 'all', omit_env: ['A\ud800B'] },
			{ env: 1 },
			{ env: [] },
			{ env: { 'A=B': 'x' } },
			{ env: { A: 'a\0b' } },
			{ env: { A: 'a\ud800b' } },
			{ env: { A: 1 } }
		];
		for (const options of bad) assert_throws_type(() => env_program(spawn, options));
	});
});

describe('handles', () => {
	const fake_receivers = (): Array<object> => [
		{ path: '/bin/sh', name: 'sh' },
		Object.create(Object.getPrototypeOf(spawn), { path: { value: '/bin/sh' } })
	];

	test('the Object functions it calls are copied at load', () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const original_assign = Object.assign;
		const original_create = Object.create;
		const forbidden = (): never => {
			throw new Error('patched after load');
		};
		let program: SpawnProgram;
		Object.assign = forbidden;
		Object.create = forbidden;
		try {
			program = spawn.program('ok');
		} finally {
			Object.assign = original_assign;
			Object.create = original_create;
		}
		assert.strictEqual(program.name, 'ok');
	});

	test('the root, handles, results, and their prototypes are frozen', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		const narrowed = spawn.narrow(['ok']);
		for (const handle of [spawn, narrowed, program]) {
			assert.ok(Object.isFrozen(handle));
			const proto = Object.getPrototypeOf(handle);
			assert.ok(Object.isFrozen(proto));
			assert.ok(Object.isFrozen(proto.constructor));
			assert.throws(() => {
				proto.patched = true;
			}, TypeError);
		}
		assert.throws(() => {
			(program as any).path = '/bin/sh';
		}, TypeError);
		const result = await program.run([]);
		assert.ok(Object.isFrozen(result));
		const output = await program.out([]);
		assert.ok(Object.isFrozen(output));
		assert.throws(() => {
			(output as any).stdout = 'patched';
		}, TypeError);
	});

	test('results have no prototype, so no thenable or species trick reaches them', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		const run_result = await program.run([]);
		const out_result = await program.out([]);
		for (const result of [run_result, out_result] as Array<any>) {
			assert.isNull(Object.getPrototypeOf(result));
			assert.isUndefined(result.then);
			assert.isUndefined(result.constructor);
		}
		assert.strictEqual(typeof out_result.stdout, 'string');
		// `resolve` reads `then` synchronously, so nothing awaits while it's polluted
		(Object.prototype as any).then = function (resolve: (value: string) => void) {
			resolve('forged');
		};
		let resolved: Array<Promise<unknown>>;
		try {
			resolved = [Promise.resolve(run_result), Promise.resolve(out_result)];
		} finally {
			delete (Object.prototype as any).then;
		}
		assert.strictEqual(await resolved[0], run_result);
		assert.strictEqual(await resolved[1], out_result);
	});

	test("a handle's constructor makes no handle", () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const program = spawn.program('ok');
		const narrowed = spawn.narrow(['ok']);
		for (const handle of [spawn, narrowed, program] as Array<any>) {
			const Ctor = handle.constructor;
			const argses: Array<Array<unknown>> = [
				[],
				[Symbol('spawn handle'), null],
				[Symbol('spawn handle'), 'ok', '/bin/sh', {}]
			];
			for (const args of argses) {
				assert.throws(() => Reflect.construct(Ctor, args), TypeError);
			}
		}
	});

	test('no method acts on a borrowed receiver', async () => {
		process.env.PATH = bin;
		write_script(join(bin, 'ok'), 'exit 0');
		const handles: Record<string, object> = {
			spawn,
			narrowed: spawn.narrow(['ok']),
			program: spawn.program('ok')
		};
		const seen = new Set<string>();
		for (const [kind, handle] of Object.entries(handles)) {
			const proto = Object.getPrototypeOf(handle);
			assert.deepEqual(Object.getOwnPropertyNames(proto.constructor).sort(), [
				'length',
				'name',
				'prototype'
			]);
			for (const key of Object.getOwnPropertyNames(proto)) {
				if (key === 'constructor') continue;
				seen.add(key);
				const fn = Object.getOwnPropertyDescriptor(proto, key)!.value as (
					...a: Array<unknown>
				) => unknown;
				// the args that pass every check of the method body, so only the receiver can throw
				const argses: Array<Array<unknown>> = [
					[['ok']],
					[[]],
					['ok'],
					['ok', { pass_env: 'all', omit_env: ['A'] }]
				];
				for (const fake of fake_receivers()) {
					for (const args of argses) {
						const label = `${kind} ${key} ${JSON.stringify(args)}`;
						try {
							await fn.apply(fake, args);
							assert.fail(`expected a TypeError from ${label}`);
						} catch (error) {
							assert.instanceOf(error, TypeError, label);
						}
					}
				}
			}
		}
		for (const key of ['program', 'find', 'narrow', 'run', 'out']) {
			assert.ok(seen.has(key), `enumerated ${key}`);
		}
	});
});
