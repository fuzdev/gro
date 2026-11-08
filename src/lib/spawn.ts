/**
 * The spawn host module: starting programs through handles pinned to one executable.
 *
 * A `SpawnProgram` is the capability to run one executable, the shape a spawn grant takes on
 * the runtime. `spawn.program('git')` resolves the name once, synchronously, walking the
 * host's `PATH`: only absolute entries, the first regular executable. The real path is pinned
 * for the handle's life, so a later `PATH` change never changes which binary runs,
 * and neither `spawn.program` nor `spawn.find` looks in `node_modules/.bin`, so a dependency
 * can't shadow `git`. A name containing `/` resolves against the
 * project root instead, and one that's absolute or climbs above the project through `..`
 * is `not_granted`, a check of the name as written, since a symlink beneath the project can
 * lead anywhere. Each program runs at its pinned real path. A bare name is its own `argv[0]`,
 * because a multicall binary (`~/.cargo/bin/cargo -> rustup`) dispatches on it, and a name
 * containing `/` passes its lexical project-absolute path, so a tool that finds itself through
 * `argv[0]` (a virtualenv's `python`) still does under another `cwd`.
 *
 * A call is argv only, never a shell string, its name and arguments checked strings.
 * A file is pinned only when its first bytes are `#!` or the native binary magic of the
 * host's platform, Mach-O on macOS and ELF elsewhere, since on Linux `execvp`, which Node's
 * child start uses, hands any other file the kernel won't run to `/bin/sh`. A file with execute
 * permission this user can't read passes, as it can't be inspected, and `binfmt_misc`
 * formats are refused. `.bat` and `.cmd` are refused,
 * as what `cmd.exe` would interpret. A non-zero exit is a result;
 * `SpawnError` (`spawn_error.ts`) means a program couldn't be found or started. Handles come only from
 * this module, hold their pin, `argv[0]`, and environment in private fields, and are frozen
 * with their prototypes, so a method called on any other object throws `TypeError`.
 * `spawn.narrow(names)` keeps only named programs, and narrowing can only shrink.
 * A narrowed root refuses `env`, which turns a named program into arbitrary execution
 * (`LD_PRELOAD`), and `pass_env: 'all'` and `pass_env` globs, so a holder of a narrowed root
 * can't take the whole environment. Named `pass_env` stays, forwarding one host value at a
 * time. To give a library a program with an environment, build the `SpawnProgram` at the
 * root and hand that down.
 *
 * A child's environment is cleared, the rules of the runtime's grant: `PATH`, `HOME`, `TMPDIR`,
 * and `TERM` when the host has them, `LC_ALL=C`, the host's own `NO_COLOR` and `FORCE_COLOR`, then
 * the names `pass_env` lists, then the values `env` gives, all fixed when the handle is made,
 * never per call. `pass_env: 'all'` is the hatch that hands over the host's whole environment,
 * as strong as `-A`, so use it only where a person chose it, and review it as such.
 * Option objects are read once, each option's value read once into a null-prototype object
 * and checked, so a getter can't change one between its check and its use. Only own
 * properties count as options, so a polluted `Object.prototype` can't set one on a plain `{}`.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The pin is a path, resolved then exec'd later by path: a file replaced at that path
 *   in between runs in place of what was pinned. The runtime's builtin execs a descriptor.
 * - `cwd` is read as a path from anything shaped like a handle, its `path` a checked string,
 *   so a forged handle object names any directory. The runtime's builtin takes the handle's
 *   descriptor, which only a real handle has.
 * - A real `cwd` handle follows its path too: where `fs` refuses a directory that was replaced
 *   (`outside_root`, renamed away and a symlink left at the path), a call runs in what
 *   the symlink leads to.
 * - No policy check, argv limit, or kernel confinement applies to the children: a running
 *   program reaches whatever its user can.
 * - Intrinsics stay pollutable, as for fs. The module copies at load the Node functions it calls,
 *   `Array.isArray`, `Number.isSafeInteger`, and `Object`'s `assign`, `create`, `freeze`, `keys`, and
 *   `hasOwn`, and its own objects are null-prototype, but code that patches `Array.prototype`,
 *   `String.prototype`, `Promise`, `Uint8Array`, `TextDecoder`, or `Error` after this module loads
 *   runs inside these ops.
 * - Any module can import `node:child_process`, so a narrowed `spawn` bounds only code
 *   that uses it, a review signal, not a boundary.
 * - The refusal of files with no `#!` reads only the first bytes, so a file with a native magic
 *   the kernel won't load, or a `#!` line naming no interpreter, still reaches Linux's
 *   `/bin/sh` fallback, and a Java class file passes on macOS, sharing the fat binary's magic.
 * - A shebang script's `$0` is the pinned path, not the name it was granted, so a shell script
 *   that dispatches on the symlink it was called through can't.
 * - A child killed by a real-time signal is reported as `{kind: 'exited', code: 0}`,
 *   since Node passes an empty signal name for one.
 * - Node copies a set `NODE_V8_COVERAGE` into every child's environment.
 * - Importing this module throws in a removed or non-UTF-8 working directory, as `fs` does.
 *
 * @module
 */

import * as node_child_process from 'node:child_process';
import * as node_fs from 'node:fs';
import * as node_path from 'node:path';
import type { ChildProcess, SpawnOptions } from 'node:child_process';

import type { FsReadDir } from './fs.ts';
import { to_checked_cwd } from './internal/cwd.ts';
import { SpawnError } from './spawn_error.ts';

// copied at load rather than imported by name, since a named import of a builtin is a live
// binding that `syncBuiltinESMExports` rebinds to whatever a caller assigned to the export
const { spawn: node_spawn } = node_child_process;
const { accessSync, closeSync, constants: fs_constants, openSync, readSync, statSync } = node_fs;
const { O_NONBLOCK, O_RDONLY, X_OK } = fs_constants;
const realpath_sync_native = node_fs.realpathSync.native;
const { delimiter, isAbsolute } = node_path;
const { isArray } = Array;
const { isSafeInteger } = Number;
const { assign, create, freeze, keys, hasOwn } = Object;
const host_env = process.env;
const { platform: host_platform } = process;

/**
 * The project root, where a name containing `/` resolves and a call's cwd defaults to,
 * the same source as `fs.project`, without this module linking `fs`.
 */
const PROJECT_ROOT = to_checked_cwd();

/**
 * The default cap on each of a captured call's stdout and stderr.
 */
const DEFAULT_MAX_BYTES = 64 * 1024 * 1024;

// fatal is off: `out` is the text view of output, and U+FFFD is what bad bytes become in it
const output_decoder = new TextDecoder('utf-8', { ignoreBOM: true });

// TODO: `start` (a `SpawnChild` with stdin, output iterables, and kill), `signal`, and `timeout_ms`,
// and child cleanup on the host's exit

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('spawn handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('spawn handles come from `spawn` and other handles, not a constructor');
	}
};

// names a narrowed root allows, null-prototype so no name resolves through `Object.prototype`
type Allowed = Readonly<Record<string, true | undefined>>;

/**
 * How a program's environment is built, fixed when its handle is made.
 */
export interface SpawnProgramOptions {
	/**
	 * Host variables to pass by name, beyond the base set: a name or a `PREFIX_*` glob.
	 * `'all'` passes the host's whole environment, as strong as `-A`.
	 * A narrowed root takes names only.
	 */
	pass_env?: ReadonlyArray<string> | 'all';
	/**
	 * Explicit values, the last word over the base set and `pass_env`.
	 * Given only when the handle is made: a call can't change a program's environment.
	 */
	env?: Readonly<Record<string, string>>;
}

/**
 * Options for a call that starts a program.
 */
export interface SpawnCallOptions {
	/**
	 * The working directory, a handle rather than a path. Defaults to the project root.
	 */
	cwd?: FsReadDir;
}

/**
 * Options for a captured call.
 */
export interface SpawnOutOptions extends SpawnCallOptions {
	/**
	 * The most bytes kept of each of stdout and stderr, a non-negative safe integer.
	 * Past it the child is killed and the result is `truncated`. Defaults to 64 MiB.
	 */
	max_bytes?: number;
}

/**
 * How a program ended. `code` is its exit code, or `signal` the signal that ended it.
 */
export type SpawnResult =
	| { readonly kind: 'exited'; readonly code: number }
	| { readonly kind: 'signaled'; readonly signal: string };

/**
 * A captured call's result. `stdout` and `stderr` decode as UTF-8, with `U+FFFD` for
 * bytes that aren't, each at most `max_bytes` before decoding.
 *
 * When `truncated` is `true`, output passed `max_bytes` and the child was killed
 * with `SIGKILL`: its `kind` is then `signaled` unless it exited first.
 */
export type SpawnOutput = SpawnResult & {
	readonly stdout: string;
	readonly stderr: string;
	readonly truncated: boolean;
};

/**
 * One executable, pinned when the handle is made. `name` and `path` are for display:
 * the pin is private, so changing what `path` says changes nothing that runs.
 */
export interface SpawnProgram {
	/**
	 * The name the program was granted. A bare name is also its `argv[0]`,
	 * and a name containing `/` passes its project-absolute path instead.
	 */
	readonly name: string;
	/**
	 * The pinned executable's real path, for display.
	 */
	readonly path: string;
	/**
	 * Runs the program with stdio inherited, resolving when it ends.
	 * A non-zero exit is a result, not an error.
	 * @param args - the arguments after `argv[0]`
	 * @throws `TypeError` - for an argument that isn't a well-formed string without NUL
	 * @throws `SpawnError` - `launch_failed`
	 */
	run(args: ReadonlyArray<string>, options?: SpawnCallOptions): Promise<SpawnResult>;
	/**
	 * Runs the program with stdin from `/dev/null`, capturing stdout and stderr as UTF-8 text.
	 * Output past `max_bytes` (64 MiB by default) on either stream kills the child with
	 * `SIGKILL`, whose result is `truncated`, a multi-byte character cut at the end
	 * becoming `U+FFFD`.
	 * @param args - the arguments after `argv[0]`
	 * @throws `TypeError` - for an argument that isn't a well-formed string without NUL
	 * @throws `SpawnError` - `launch_failed`
	 */
	out(args: ReadonlyArray<string>, options?: SpawnOutOptions): Promise<SpawnOutput>;
}

/**
 * The spawn host module's root: makes program handles and narrower roots.
 */
export interface Spawn {
	/**
	 * Resolves `name` and pins it for the handle's life.
	 * A bare name walks the host's `PATH`, a name containing `/` resolves against the project root.
	 * Options are read and checked before the name resolves.
	 * @throws `TypeError` - for a name that isn't a non-empty, well-formed string without NUL,
	 * or a malformed option
	 * @throws `SpawnError` - `not_found`, `not_executable`, `not_granted`, the last also for `env`,
	 * `pass_env: 'all'`, or a `pass_env` glob on a narrowed root
	 */
	program(name: string, options?: SpawnProgramOptions): SpawnProgram;
	/**
	 * Like `program`, returning `null` when nothing is found.
	 * @returns the handle, or `null` on `not_found`
	 * @throws `TypeError` - as `program` does
	 * @throws `SpawnError` - `not_executable`, `not_granted`
	 */
	find(name: string, options?: SpawnProgramOptions): SpawnProgram | null;
	/**
	 * A root that resolves only `names`, each spelled as it will be requested.
	 * It can only shrink: a name this root doesn't allow throws rather than widens.
	 * The root it returns refuses `env`, `pass_env: 'all'`, and `pass_env` globs.
	 * @throws `TypeError` - for a name that isn't a non-empty, well-formed string without NUL
	 * @throws `SpawnError` - `not_granted`
	 */
	narrow(names: ReadonlyArray<string>): Spawn;
}

/**
 * A string with no NUL byte and well-formed Unicode, whose lone surrogates would otherwise
 * reach a child as U+FFFD.
 */
const check_text = (value: unknown, what: string): string => {
	if (typeof value !== 'string') throw new TypeError(`${what} must be a string`);
	if (value.includes('\0')) throw new TypeError(`${what} holds a NUL byte`);
	if (!value.isWellFormed()) throw new TypeError(`${what} isn't well-formed Unicode`);
	return value;
};

const check_nonempty_text = (value: unknown, what: string): string => {
	const text = check_text(value, what);
	if (text.length === 0) throw new TypeError(`${what} is empty`);
	return text;
};

const check_name = (name: unknown): string => check_nonempty_text(name, 'a spawn program name');

/**
 * Whether `path` ends with a Windows-shell extension, allowing the trailing dots and spaces
 * Windows strips from a filename.
 */
const is_refused_extension = (path: string): boolean => /\.(?:bat|cmd)[. ]*$/i.test(path);

// a null-prototype record, so no name resolves through `Object.prototype`
const create_record = <T extends object>(fields: T): T => assign(create(null) as T, fields);

/**
 * An option's value, read as an own property so a polluted `Object.prototype` can't set one.
 */
const read_own = (options: object | null | undefined, key: string): unknown =>
	options !== null && options !== undefined && hasOwn(options, key)
		? (options as Record<string, unknown>)[key]
		: undefined;

type Probe = 'executable' | 'not_executable' | 'missing';

/**
 * What's at `path` as a program: a regular file with execute permission, one without, or neither.
 * A failed stat or an unusable entry is `missing`, so a path entry this user can't read
 * doesn't stop a `PATH` walk, as `execvp` continues past one.
 */
const probe_executable = (path: string): Probe => {
	let info;
	try {
		info = statSync(path, { throwIfNoEntry: false });
	} catch {
		return 'missing';
	}
	if (info === undefined || !info.isFile()) return 'missing';
	try {
		accessSync(path, X_OK);
		return 'executable';
	} catch {
		return 'not_executable';
	}
};

const MAGIC_LENGTH = 4;

const ELF_MAGIC = 0x7f454c46;

// as big-endian words of the bytes on disk: 32-bit and 64-bit in each byte order, and fat
const is_mach_o_magic = (magic: number): boolean =>
	magic === 0xfeedface ||
	magic === 0xfeedfacf ||
	magic === 0xcefaedfe ||
	magic === 0xcffaedfe ||
	magic === 0xcafebabe;

/**
 * Whether a file's first bytes are `#!` or the platform's native binary magic, the formats
 * the kernel runs without `execvp` falling back to `/bin/sh`.
 */
const is_runnable_magic = (bytes: Uint8Array, platform: string): boolean => {
	if (bytes[0] === 0x23 && bytes[1] === 0x21) return true;
	const magic = ((bytes[0]! << 24) | (bytes[1]! << 16) | (bytes[2]! << 8) | bytes[3]!) >>> 0;
	return platform === 'darwin' ? is_mach_o_magic(magic) : magic === ELF_MAGIC;
};

/**
 * Refuses a pinned file that would reach `execvp`'s `/bin/sh` fallback.
 * One this user can't read passes, since it can't be inspected. Non-blocking,
 * so a file swapped for a FIFO since its stat can't hang the open.
 * @throws `SpawnError` - `not_executable`
 */
const check_runnable_format = (name: string, path: string): void => {
	let fd: number;
	try {
		fd = openSync(path, O_RDONLY | O_NONBLOCK);
	} catch (error) {
		if ((error as { code?: unknown }).code === 'EACCES') return;
		throw to_uninspectable_error(name, path, error);
	}
	// zeroed, so a file shorter than the magic can't match it
	const bytes = new Uint8Array(MAGIC_LENGTH);
	try {
		readSync(fd, bytes, 0, MAGIC_LENGTH, 0);
	} catch (error) {
		throw to_uninspectable_error(name, path, error);
	} finally {
		closeSync(fd);
	}
	if (!is_runnable_magic(bytes, host_platform)) {
		throw new SpawnError(
			'not_executable',
			name,
			`not_executable: ${name} at ${path} starts with neither #! nor a native binary's magic`
		);
	}
};

const to_uninspectable_error = (name: string, path: string, error: unknown): SpawnError =>
	new SpawnError('not_executable', name, `not_executable: ${name} at ${path} can't be read`, {
		cause: error
	});

// `fatal`, so a real path that isn't UTF-8 is refused rather than naming a different file
const real_path_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/**
 * The real path to pin for `path`, a found executable.
 * @throws `SpawnError` - `not_executable`
 */
const to_pin = (name: string, path: string): string => {
	let real: string;
	try {
		real = real_path_decoder.decode(realpath_sync_native(path, { encoding: 'buffer' }));
	} catch (error) {
		throw new SpawnError(
			'not_executable',
			name,
			`not_executable: ${name} at ${path} has no real path a string can name`,
			{ cause: error }
		);
	}
	if (is_refused_extension(real)) throw to_refused_extension_error(name, real);
	check_runnable_format(name, real);
	return real;
};

const to_refused_extension_error = (name: string, path: string): SpawnError =>
	new SpawnError(
		'not_executable',
		name,
		`not_executable: ${name} at ${path} is a .bat or .cmd file, which only a shell runs`
	);

/**
 * Finds the first regular executable named `name` on the host's `PATH`, absolute entries only,
 * so an empty or relative entry can't reach the working directory.
 * @returns the entry's path, or `null` when no entry has one
 * @throws `SpawnError` - `not_executable`
 */
const walk_path = (name: string): string | null => {
	const value = hasOwn(host_env, 'PATH') ? host_env.PATH : undefined;
	if (value === undefined) return null;
	const entries = value.split(delimiter);
	let found_not_executable = false;
	for (let i = 0; i < entries.length; i++) {
		const entry = entries[i]!;
		if (!isAbsolute(entry)) continue;
		const candidate = entry.endsWith('/') ? entry + name : entry + '/' + name;
		const probed = probe_executable(candidate);
		if (probed === 'executable') return candidate;
		if (probed === 'not_executable') found_not_executable = true;
	}
	if (found_not_executable) {
		throw new SpawnError(
			'not_executable',
			name,
			`not_executable: ${name} is on PATH only without execute permission`
		);
	}
	return null;
};

/**
 * Resolves a name containing `/` against the project root, lexically.
 * @throws `SpawnError` - `not_granted` for an absolute name or one climbing above the project
 */
const to_project_path = (name: string): string => {
	if (name.startsWith('/')) {
		throw new SpawnError(
			'not_granted',
			name,
			`not_granted: ${name} is absolute, and a program name with / resolves beneath the project`
		);
	}
	const segments: Array<string> = [];
	const parts = name.split('/');
	for (let i = 0; i < parts.length; i++) {
		const segment = parts[i]!;
		if (segment === '' || segment === '.') continue;
		if (segment === '..') {
			if (segments.length === 0) {
				throw new SpawnError(
					'not_granted',
					name,
					`not_granted: ${name} climbs above the project ${PROJECT_ROOT}`
				);
			}
			segments.length -= 1;
		} else {
			segments[segments.length] = segment;
		}
	}
	const prefix = PROJECT_ROOT === '/' ? '/' : PROJECT_ROOT + '/';
	return prefix + segments.join('/');
};

/**
 * What a name resolved to: the real path to run and the `argv[0]` to pass.
 */
interface ResolvedProgram {
	readonly pin: string;
	readonly argv0: string;
}

/**
 * Resolves a program name to what to run, or `null` for `not_found`.
 * @throws `SpawnError` - `not_granted`, `not_executable`
 */
const resolve_program = (allowed: Allowed | null, name: string): ResolvedProgram | null => {
	if (allowed !== null && allowed[name] !== true) {
		throw new SpawnError('not_granted', name, `not_granted: ${name} isn't in this spawn's set`);
	}
	if (is_refused_extension(name)) throw to_refused_extension_error(name, name);
	if (!name.includes('/')) {
		const found = walk_path(name);
		return found === null ? null : { pin: to_pin(name, found), argv0: name };
	}
	const path = to_project_path(name);
	// a trailing `/` or `/.` passes through its last component, which only a directory can be
	if (name.endsWith('/') || name.endsWith('/.')) return null;
	const probed = probe_executable(path);
	if (probed === 'missing') return null;
	if (probed === 'not_executable') {
		throw new SpawnError('not_executable', name, `not_executable: ${path} isn't executable`);
	}
	return { pin: to_pin(name, path), argv0: path };
};

const check_env_name = (value: unknown, what: string): string => {
	const name = check_nonempty_text(value, what);
	if (name.includes('=')) throw new TypeError(`${what} holds a \`=\`: ${name}`);
	return name;
};

/**
 * Reads a `pass_env` option once, element by element, refusing a malformed entry.
 */
const read_pass_env = (raw: unknown): ReadonlyArray<string> | 'all' => {
	if (raw === undefined) return [];
	if (raw === 'all') return 'all';
	if (!isArray(raw)) throw new TypeError("`pass_env` is an array of names or 'all'");
	const patterns: Array<string> = [];
	const length = raw.length;
	for (let i = 0; i < length; i++) {
		const pattern = check_env_name(raw[i], 'a `pass_env` entry');
		const star = pattern.indexOf('*');
		if (star !== -1 && star !== pattern.length - 1) {
			throw new TypeError(`a \`pass_env\` glob ends in its one \`*\`: ${pattern}`);
		}
		// `*` alone would be `'all'` without saying so, which the policy diff must show
		if (star === 0) throw new TypeError("`pass_env` glob needs a prefix: use 'all'");
		patterns[patterns.length] = pattern;
	}
	return patterns;
};

const copy_host_var = (env: Record<string, string>, name: string): void => {
	if (hasOwn(host_env, name)) {
		const value = host_env[name];
		if (typeof value === 'string') env[name] = value;
	}
};

const copy_base_env = (env: Record<string, string>): void => {
	copy_host_var(env, 'PATH');
	copy_host_var(env, 'HOME');
	copy_host_var(env, 'TMPDIR');
	copy_host_var(env, 'TERM');
	env.LC_ALL = 'C';
	copy_host_var(env, 'NO_COLOR');
	copy_host_var(env, 'FORCE_COLOR');
};

const copy_all_env = (env: Record<string, string>): void => {
	const names = keys(host_env);
	for (let i = 0; i < names.length; i++) copy_host_var(env, names[i]!);
};

const copy_pass_env = (env: Record<string, string>, patterns: ReadonlyArray<string>): void => {
	for (let i = 0; i < patterns.length; i++) {
		const pattern = patterns[i]!;
		if (pattern.endsWith('*')) {
			const prefix = pattern.slice(0, -1);
			const names = keys(host_env);
			for (let j = 0; j < names.length; j++) {
				if (names[j]!.startsWith(prefix)) copy_host_var(env, names[j]!);
			}
		} else {
			copy_host_var(env, pattern);
		}
	}
};

/**
 * Reads an `env` option once, key by key, refusing a malformed name or value.
 * An empty value is a value; a variable set to nothing is legitimate.
 * @returns the values, or `null` when the option wasn't given
 */
const read_env_values = (raw: unknown): Record<string, string> | null => {
	if (raw === undefined) return null;
	const values = create_record<Record<string, string>>({});
	if (typeof raw !== 'object' || raw === null || isArray(raw)) {
		throw new TypeError('`env` is a record of names to values');
	}
	const names = keys(raw);
	for (let i = 0; i < names.length; i++) {
		const name = check_env_name(names[i], 'an `env` name');
		values[name] = check_text(
			(raw as Record<string, unknown>)[name],
			`the \`env\` value of ${name}`
		);
	}
	return values;
};

/**
 * A program's options, each read once into a null-prototype object.
 */
interface ProgramOptions {
	readonly pass_env: ReadonlyArray<string> | 'all';
	readonly values: Record<string, string> | null;
}

/**
 * Reads and checks a program's options once, before its name resolves.
 */
const read_program_options = (options: SpawnProgramOptions | undefined): ProgramOptions => {
	if (
		options !== undefined &&
		(typeof options !== 'object' || options === null || isArray(options))
	) {
		throw new TypeError('spawn program options are an object');
	}
	const pass_env = read_pass_env(read_own(options, 'pass_env'));
	const values = read_env_values(read_own(options, 'env'));
	return create_record({ pass_env, values });
};

/**
 * Refuses on a narrowed root what would make a named program arbitrary execution (`env`)
 * or hand its holder the whole environment (`pass_env: 'all'` and globs, which enumerate it).
 * @throws `SpawnError` - `not_granted`
 */
const check_narrowed_options = (
	allowed: Allowed | null,
	name: string,
	options: ProgramOptions
): void => {
	if (allowed === null) return;
	if (options.values !== null) {
		throw new SpawnError(
			'not_granted',
			name,
			`not_granted: a narrowed spawn can't set env for ${name}: build the program at the root`
		);
	}
	if (options.pass_env === 'all') {
		throw new SpawnError(
			'not_granted',
			name,
			`not_granted: a narrowed spawn can't pass the whole environment to ${name}`
		);
	}
	for (let i = 0; i < options.pass_env.length; i++) {
		const pattern = options.pass_env[i]!;
		if (pattern.endsWith('*')) {
			throw new SpawnError(
				'not_granted',
				name,
				`not_granted: a narrowed spawn can't pass the glob ${pattern} to ${name}: name each variable`
			);
		}
	}
};

/**
 * Builds a program's environment once, at creation, from its options read earlier.
 */
const to_program_env = ({ pass_env, values }: ProgramOptions): Record<string, string> => {
	const env = create_record<Record<string, string>>({});
	if (pass_env === 'all') {
		copy_all_env(env);
	} else {
		copy_base_env(env);
		copy_pass_env(env, pass_env);
	}
	if (values !== null) {
		const names = keys(values);
		for (let i = 0; i < names.length; i++) {
			const name = names[i]!;
			env[name] = values[name]!;
		}
	}
	return env;
};

const check_call_options = (
	options: SpawnCallOptions | undefined
): Readonly<Record<string, unknown>> | null => {
	if (options === undefined) return null;
	if (typeof options !== 'object' || options === null || isArray(options)) {
		throw new TypeError('spawn call options are an object');
	}
	return options as Record<string, unknown>;
};

/**
 * Reads a `cwd` option's handle, once, for its path.
 * A forged object with a string `path` passes, the documented gap.
 */
const read_cwd = (raw: unknown): string => {
	if (raw === undefined) return PROJECT_ROOT;
	if (typeof raw !== 'object' || raw === null) {
		throw new TypeError('`cwd` is an `fs` directory handle');
	}
	const path = check_nonempty_text((raw as { path?: unknown }).path, 'a `cwd` handle path');
	if (!isAbsolute(path)) throw new TypeError(`a \`cwd\` handle path is absolute: ${path}`);
	return path;
};

const to_argv = (args: ReadonlyArray<string>): Array<string> => {
	if (!isArray(args)) throw new TypeError('spawn args are an array of strings');
	const argv: Array<string> = [];
	const length = args.length;
	for (let i = 0; i < length; i++) {
		argv[argv.length] = check_text(args[i], 'a spawn argument');
	}
	return argv;
};

const to_launch_error = (name: string, pin: string, error: unknown): SpawnError => {
	const detail = error instanceof Error ? error.message : String(error);
	return new SpawnError('launch_failed', name, `launch_failed: ${name} (${pin}): ${detail}`, {
		cause: error
	});
};

/**
 * Starts the program. The env is a null-prototype copy per call, since Node enumerates it
 * with `for in`, which would reach a polluted `Object.prototype.X`, and writes
 * `NODE_V8_COVERAGE` into the object it's given. The options are null-prototype too, on top
 * of Node's own null-prototype copy of them.
 */
const launch = (
	name: string,
	argv0: string,
	pin: string,
	env: Record<string, string>,
	argv: Array<string>,
	cwd: string,
	stdio: SpawnOptions['stdio']
): ChildProcess => {
	try {
		return node_spawn(
			pin,
			argv,
			create_record<SpawnOptions>({
				argv0,
				cwd,
				env: create_record<Record<string, string>>({ ...env }),
				shell: false,
				stdio
			})
		);
	} catch (error) {
		throw to_launch_error(name, pin, error);
	}
};

const to_result = (code: number | null, signal: string | null): SpawnResult =>
	freeze(
		signal !== null
			? create_record<SpawnResult>({ kind: 'signaled', signal })
			: // `close` always has a code or a signal for a process that started
				create_record<SpawnResult>({ kind: 'exited', code: code! })
	);

/**
 * Settles when `child` closes. An `error` only rejects before the child started: Node also emits
 * `error` for a failed `kill` of a running child, which isn't a launch failure, and a
 * failed launch emits `close` too, which the first settlement already handled.
 */
const to_closed = (child: ChildProcess, name: string, pin: string): Promise<SpawnResult> =>
	new Promise((resolve, reject) => {
		child.on('error', (error) => {
			if (child.pid === undefined) reject(to_launch_error(name, pin, error));
		});
		child.on('close', (code, signal) => {
			resolve(to_result(code, signal));
		});
	});

interface Capture {
	readonly chunks: Array<Uint8Array>;
	size: number;
}

const to_captured_text = (capture: Capture): string => {
	const bytes = new Uint8Array(capture.size);
	let offset = 0;
	for (let i = 0; i < capture.chunks.length; i++) {
		const chunk = capture.chunks[i]!;
		bytes.set(chunk, offset);
		offset += chunk.length;
	}
	return output_decoder.decode(bytes);
};

/**
 * Stores `chunk`, keeping at most `max_bytes` of the stream.
 * @returns whether the stream went past `max_bytes`, in which case the rest was dropped
 */
const add_chunk = (capture: Capture, chunk: Uint8Array, max_bytes: number): boolean => {
	const room = max_bytes - capture.size;
	const exceeded = chunk.length > room;
	const kept = exceeded ? chunk.subarray(0, room) : chunk;
	capture.chunks[capture.chunks.length] = kept;
	capture.size += kept.length;
	return exceeded;
};

/**
 * A captured call's options, read once into a null-prototype object.
 */
const read_out_options = (
	options: SpawnOutOptions | undefined
): { cwd: string; max_bytes: number } => {
	const raw = check_call_options(options);
	const cwd = read_cwd(read_own(raw, 'cwd'));
	const max_bytes = read_own(raw, 'max_bytes');
	if (max_bytes === undefined) return create_record({ cwd, max_bytes: DEFAULT_MAX_BYTES });
	if (typeof max_bytes !== 'number' || !isSafeInteger(max_bytes) || max_bytes < 0) {
		throw new TypeError('`max_bytes` is a non-negative safe integer');
	}
	return create_record({ cwd, max_bytes });
};

class NodeSpawnProgram implements SpawnProgram {
	readonly name: string;
	readonly path: string;
	readonly #pin: string;
	readonly #argv0: string;
	readonly #env: Record<string, string>;

	constructor(key: symbol, name: string, pin: string, argv0: string, env: Record<string, string>) {
		check_handle_key(key);
		this.name = name;
		this.path = pin;
		this.#pin = pin;
		this.#argv0 = argv0;
		this.#env = env;
	}

	async run(args: ReadonlyArray<string>, options?: SpawnCallOptions): Promise<SpawnResult> {
		const pin = this.#pin;
		const argv = to_argv(args);
		const cwd = read_cwd(read_own(check_call_options(options), 'cwd'));
		const child = launch(this.name, this.#argv0, pin, this.#env, argv, cwd, 'inherit');
		return to_closed(child, this.name, pin);
	}

	async out(args: ReadonlyArray<string>, options?: SpawnOutOptions): Promise<SpawnOutput> {
		const pin = this.#pin;
		const argv = to_argv(args);
		const { cwd, max_bytes } = read_out_options(options);
		const child = launch(this.name, this.#argv0, pin, this.#env, argv, cwd, [
			'ignore',
			'pipe',
			'pipe'
		]);
		const stdout: Capture = { chunks: [], size: 0 };
		const stderr: Capture = { chunks: [], size: 0 };
		let truncated = false;
		const collect = (capture: Capture) => (chunk: Uint8Array) => {
			if (truncated || !add_chunk(capture, chunk, max_bytes)) return;
			truncated = true;
			// the streams are destroyed too, so a grandchild holding them open can't hold `close`
			child.kill('SIGKILL');
			child.stdout?.destroy();
			child.stderr?.destroy();
		};
		child.stdout?.on('data', collect(stdout));
		child.stderr?.on('data', collect(stderr));
		const result = await to_closed(child, this.name, pin);
		return freeze(
			create_record({
				...result,
				stdout: to_captured_text(stdout),
				stderr: to_captured_text(stderr),
				truncated
			})
		);
	}
}

/**
 * Makes the handle for `raw_name`, options read and checked before the name resolves.
 * @returns the handle, or `null` on `not_found`
 */
const to_program = (
	allowed: Allowed | null,
	raw_name: string,
	raw_options: SpawnProgramOptions | undefined
): SpawnProgram | null => {
	const name = check_name(raw_name);
	const options = read_program_options(raw_options);
	check_narrowed_options(allowed, name, options);
	const resolved = resolve_program(allowed, name);
	if (resolved === null) return null;
	return freeze(
		new NodeSpawnProgram(HANDLE_KEY, name, resolved.pin, resolved.argv0, to_program_env(options))
	);
};

class NodeSpawn implements Spawn {
	readonly #allowed: Allowed | null;

	constructor(key: symbol, allowed: Allowed | null) {
		check_handle_key(key);
		this.#allowed = allowed;
	}

	program(name: string, options?: SpawnProgramOptions): SpawnProgram {
		const program = to_program(this.#allowed, name, options);
		if (program === null) throw new SpawnError('not_found', name, `not_found: no program ${name}`);
		return program;
	}

	find(name: string, options?: SpawnProgramOptions): SpawnProgram | null {
		return to_program(this.#allowed, name, options);
	}

	narrow(names: ReadonlyArray<string>): Spawn {
		const parent = this.#allowed;
		if (!isArray(names)) throw new TypeError('spawn.narrow takes an array of names');
		const allowed = create_record<Record<string, true>>({});
		const length = names.length;
		for (let i = 0; i < length; i++) {
			const name = check_name(names[i]);
			if (parent !== null && parent[name] !== true) {
				throw new SpawnError(
					'not_granted',
					name,
					`not_granted: ${name} isn't in this spawn's set, and narrow only shrinks`
				);
			}
			allowed[name] = true;
		}
		return freeze(new NodeSpawn(HANDLE_KEY, freeze(allowed)));
	}
}

// frozen, so one holder of a handle can't patch the ops another holder calls
for (const handle_class of [NodeSpawn, NodeSpawnProgram]) {
	freeze(handle_class.prototype);
	freeze(handle_class);
}

/**
 * The spawn host module's root handle. Import it where authority enters
 * (a task, a script, an entry module) and hand a narrower `spawn.narrow(...)`
 * or a single `SpawnProgram` down.
 */
export const spawn: Spawn = freeze(new NodeSpawn(HANDLE_KEY, null));
