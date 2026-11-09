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
 * `SpawnError` (`spawn_error.ts`) means a program couldn't be found or started, a call whose
 * `signal` had already aborted included, which starts nothing. Handles come only from
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
 * `omit_env` beside it leaves out exact names, case-sensitive and never a glob.
 *
 * `start` is a long-lived `SpawnChild`, `timeout_ms` and `signal` end a call's child
 * (`SIGTERM`, then `SIGKILL` after a grace period), and a call ended that way resolves to a
 * result of kind `timed_out` or `aborted`, never a rejection. A child with no inherited stdio
 * is the leader of its own process group, in a new session, so a stop, the cap, and the drain
 * bound reach what it left behind. A child with an inherited stream stays in the terminal's
 * session and foreground group, since a new session would lose the terminal: `/dev/tty` prompts
 * and Ctrl-C reaching it. Such a child is stopped by pid alone. After the direct child exits,
 * a call waits a bounded time for its pipes to close, then kills the group and stops reading,
 * so a background grandchild holding a pipe can't hang it. A reader behind at that bound keeps
 * what's buffered and can still read the rest. A stop's `SIGKILL` still reaches the group after
 * the leader exits. A background process with its pipes redirected holds nothing the call waits
 * on, so when no stop has begun it outlives the call. A child still running when
 * the host exits is killed, by one hook installed on the first spawn, and while a child in
 * a group of its own runs, the signals that end the host, but for `SIGQUIT`, kill its group first.
 * A child with an inherited stream is sent a `SIGTERM` of the host, so a chain of hosts passes it
 * all the way down.
 *
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
 * - Grouping is Node's `detached`, which is `setsid`: a child with no inherited stdio has no
 *   controlling terminal, so a program prompting on `/dev/tty` under `out` can't.
 *   A child with an inherited stream isn't grouped, so a stop reaches it by pid alone and
 *   anything it left behind survives. Node has no `PDEATHSIG`, so a `SIGKILL` of the host leaves
 *   every child behind, and a host killed by `SIGINT` or `SIGHUP` leaves an inheriting
 *   child to what the terminal delivers. Passing `SIGINT` on would double a terminal's, which
 *   makes some programs force-quit, and a listener for `SIGHUP` would end a `nohup` host.
 * - `SIGQUIT` isn't hooked, as a listener would keep a host stuck in synchronous code from
 *   dumping core, the signal's use, so a `SIGQUIT` of the host leaves a group running too.
 * - Node can't read a signal's disposition, so while a group runs, a `SIGHUP` a `nohup` host
 *   would ignore reaches the signal hook, kills the group, and ends the host.
 * - The signal hook steps aside for any other listener on the signal but another copy of itself,
 *   so a library that stands down the same way when it sees another listener (`signal-exit`)
 *   defers to the hook while the hook defers to it, and the signal ends nothing. A `once`
 *   listener prepended after a group started has removed itself before the hook counts it.
 * - A group id is signaled after its leader has been reaped, by the drain bound, a stop's grace
 *   kill, and the exit and signal hooks, so a kill could in theory reach a group the kernel handed that id to
 *   after a full wrap of pids.
 * - Intrinsics stay pollutable, as for fs. The module copies at load the Node functions it calls,
 *   `Array.isArray`, `Number.isSafeInteger`, the `process` functions, the timers, and `Object`'s
 *   `assign`, `create`, `freeze`, `keys`, and `hasOwn`, and its own objects are null-prototype, but
 *   code that patches `Array.prototype`, `Set.prototype`, `String.prototype`, `Promise`, `Uint8Array`,
 *   `TextDecoder`, `Error`, `EventEmitter.prototype.on`, or the stream classes after this module
 *   loads runs inside these ops, and a patched `on` or stream method holds the raw `ChildProcess`
 *   or stream.
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
import * as node_events from 'node:events';
import * as node_fs from 'node:fs';
import * as node_path from 'node:path';
import * as node_util from 'node:util';
import type { ChildProcess, SpawnOptions } from 'node:child_process';
import type { Readable, Writable } from 'node:stream';

import type { FsReadDir } from './fs.ts';
import { to_checked_cwd } from './internal/cwd.ts';
import { SpawnError } from './spawn_error.ts';

// copied at load rather than imported by name, since a named import of a builtin is a live
// binding that `syncBuiltinESMExports` rebinds to whatever a caller assigned to the export
const { spawn: node_spawn } = node_child_process;
// a listener that `stopImmediatePropagation` can't reach, which a plain `addEventListener` isn't
const { addAbortListener } = node_events as unknown as {
	addAbortListener: (signal: AbortSignal, listener: () => void) => Disposable;
};
const { isUint8Array } = node_util.types;
const { accessSync, closeSync, constants: fs_constants, openSync, readSync, statSync } = node_fs;
const { O_NONBLOCK, O_RDONLY, X_OK } = fs_constants;
const realpath_sync_native = node_fs.realpathSync.native;
const { delimiter, isAbsolute } = node_path;
const { isArray } = Array;
const { isSafeInteger } = Number;
const { assign, create, freeze, keys, hasOwn, setPrototypeOf } = Object;
const host_env = process.env;
const { pid: host_pid, platform: host_platform } = process;
const process_on = process.on.bind(process);
const process_prepend_listener = process.prependListener.bind(process);
const process_off = process.off.bind(process);
const process_raw_listeners = process.rawListeners.bind(process);
const process_kill = process.kill.bind(process);
const set_timer = setTimeout;
const clear_timer = clearTimeout;
const AbortSignalClass = AbortSignal;
const Uint8ArrayClass = Uint8Array;

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

/**
 * How long a call waits for a pipe to close after the direct child exits.
 */
const EXIT_DRAIN_MS = 2000;

/**
 * How long a stopped child gets between `SIGTERM` and `SIGKILL`.
 */
const STOP_GRACE_MS = 2000;

/**
 * The bytes a started child's output buffers ahead of its reader before the stream pauses.
 */
const OUTPUT_BUFFER_BYTES = 1024 * 1024;

/**
 * The longest `timeout_ms`, what `setTimeout` takes before it silently fires at once.
 */
const MAX_TIMEOUT_MS = 2147483647;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('spawn handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('spawn handles come from `spawn` and other handles, not a constructor');
	}
};

// names a narrowed root allows, null-prototype so no name resolves through `Object.prototype`
type Allowed = Readonly<Record<string, true | undefined>>;

// the names `omit_env` leaves out, null-prototype for the same reason
type OmittedNames = Readonly<Record<string, true | undefined>>;

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
	 * Host variables `pass_env: 'all'` leaves out, each an exact, case-sensitive name, never a glob.
	 * Given with any other `pass_env` it's a `TypeError`.
	 * Only what's forwarded changes, so `env` can still set one of these names, and Node still
	 * adds what it copies itself, like `NODE_V8_COVERAGE`.
	 */
	omit_env?: ReadonlyArray<string>;
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
	/**
	 * Stops the child this many milliseconds after it starts, `SIGTERM` then `SIGKILL` after a
	 * grace period. The call's result is then of kind `timed_out`.
	 * A positive safe integer of at most `2147483647`.
	 */
	timeout_ms?: number;
	/**
	 * Stops the child when it aborts, as `timeout_ms` does, its result of kind `aborted`.
	 * A signal that had aborted before the call starts nothing: the call throws `SpawnError`.
	 */
	signal?: AbortSignal;
}

/**
 * One stream of a started child: `'inherit'` shares the host's, `'pipe'` connects it
 * to the `SpawnChild`, and `'ignore'` is `/dev/null`.
 */
export type SpawnStdio = 'inherit' | 'pipe' | 'ignore';

/**
 * Options for `start`.
 */
export interface SpawnStartOptions extends SpawnCallOptions {
	/**
	 * Each of stdin, stdout, and stderr, one value for all or the three in order. Defaults to `'inherit'`.
	 * A child with nothing inherited is the leader of its own process group.
	 */
	stdio?: SpawnStdio | readonly [SpawnStdio, SpawnStdio, SpawnStdio];
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
 * `timed_out` and `aborted` are a child the module stopped for `timeout_ms` or `signal`, with
 * the `code` or `signal` it then ended by, the other `null`, so one that exits 0 on `SIGTERM`
 * is never mistaken for one that finished.
 */
export type SpawnResult =
	| { readonly kind: 'exited'; readonly code: number }
	| { readonly kind: 'signaled'; readonly signal: string }
	| {
			readonly kind: 'timed_out' | 'aborted';
			readonly code: number | null;
			readonly signal: string | null;
	  };

/**
 * A captured call's result. `stdout` and `stderr` decode as UTF-8, with `U+FFFD` for
 * bytes that aren't, each at most `max_bytes` before decoding.
 *
 * `truncated` is `true` when output passed `max_bytes` and the child was killed
 * with `SIGKILL`, its `kind` then `signaled` unless it exited first, or when a process it left
 * running still held a pipe when the wait for it ended, so output may be missing from the end.
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
	 * A non-zero exit is a result, not an error. An inheriting child is stopped by pid alone.
	 * @param args - the arguments after `argv[0]`
	 * @throws `TypeError` - for an argument that isn't a well-formed string without NUL,
	 * or a malformed option
	 * @throws `SpawnError` - `launch_failed`, also for a `signal` already aborted
	 */
	run(args: ReadonlyArray<string>, options?: SpawnCallOptions): Promise<SpawnResult>;
	/**
	 * Runs the program with stdin from `/dev/null`, capturing stdout and stderr as UTF-8 text.
	 * Output past `max_bytes` (64 MiB by default) on either stream kills the child with
	 * `SIGKILL`, whose result is `truncated`, a multi-byte character cut at the end
	 * becoming `U+FFFD`. The child leads its own process group, so the group is what's killed.
	 * After it exits, a background process still holding a pipe gets a bounded wait, then
	 * the group is killed, the result `truncated` because output may be missing.
	 * @param args - the arguments after `argv[0]`
	 * @throws `TypeError` - for an argument that isn't a well-formed string without NUL,
	 * or a malformed option
	 * @throws `SpawnError` - `launch_failed`, also for a `signal` already aborted
	 */
	out(args: ReadonlyArray<string>, options?: SpawnOutOptions): Promise<SpawnOutput>;
	/**
	 * Starts the program and returns once it's running, the child held by the `SpawnChild`.
	 * Stdio defaults to `'inherit'`. A child with nothing inherited leads its own group.
	 * @param args - the arguments after `argv[0]`
	 * @throws `TypeError` - for an argument that isn't a well-formed string without NUL,
	 * or a malformed option
	 * @throws `SpawnError` - `launch_failed`, also for a `signal` already aborted
	 */
	start(args: ReadonlyArray<string>, options?: SpawnStartOptions): Promise<SpawnChild>;
}

/**
 * A signal a child can be sent, by name. Numbers and other names are a `TypeError`.
 */
export type SpawnSignal =
	'SIGTERM' | 'SIGKILL' | 'SIGINT' | 'SIGHUP' | 'SIGQUIT' | 'SIGUSR1' | 'SIGUSR2';

/**
 * A started child's stdin.
 */
export interface SpawnStdin {
	/**
	 * Writes `data`, a string as UTF-8, resolving once it's accepted.
	 * @returns `false` when the child closed its stdin or ended, so the data went nowhere
	 * @throws `TypeError` - for data that's neither a well-formed string nor a `Uint8Array`
	 */
	write(data: string | Uint8Array): Promise<boolean>;
	/**
	 * Ends the stream, resolving once it's flushed. Closing twice is harmless.
	 */
	close(): Promise<void>;
}

/**
 * A running child. `pid` is for display, and nothing takes a pid, so no surface here can
 * signal another process. Awaiting it does nothing, as it has no `then`.
 *
 * Disposing it with `await using` stops it, `SIGTERM` then `SIGKILL` after a grace period,
 * and waits for `closed`.
 */
export interface SpawnChild extends AsyncDisposable {
	/**
	 * The child's process id, for display only.
	 */
	readonly pid: number;
	/**
	 * Resolves when the child has ended and its pipes closed, or the wait for them was
	 * bounded after it exited. Never rejects.
	 */
	readonly closed: Promise<SpawnResult>;
	/**
	 * The child's stdout, or `null` unless `'pipe'`. Iterating is once only: a
	 * second iteration throws `TypeError`, and leaving the loop early discards the rest, the
	 * child still running. Chunks are plain `Uint8Array` copies. A stream nobody reads
	 * pauses once its buffer fills, so the child can block on a full pipe.
	 */
	readonly stdout: AsyncIterable<Uint8Array> | null;
	/**
	 * The child's stderr, as `stdout`.
	 */
	readonly stderr: AsyncIterable<Uint8Array> | null;
	/**
	 * The child's stdin, or `null` unless `'pipe'`.
	 */
	readonly stdin: SpawnStdin | null;
	/**
	 * Sends `signal`, `SIGTERM` by default, to the child, or to its group when it leads
	 * one, once per call, so `SIGTERM` then `SIGKILL` works.
	 * @returns `false` when the child has already ended and no signal was sent
	 * @throws `TypeError` - for a `signal` that isn't an allowed name
	 */
	kill(signal?: SpawnSignal): boolean;
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
	const bytes = new Uint8ArrayClass(MAGIC_LENGTH);
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
	// no echo of the text, as what follows a `=` can be a value
	if (name.includes('=')) throw new TypeError(`${what} holds a \`=\``);
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

/**
 * Reads an `omit_env` option once, element by element, into a frozen null-prototype set of names.
 * @returns the names, or `null` when the option wasn't given
 */
const read_omit_env = (
	raw: unknown,
	pass_env: ReadonlyArray<string> | 'all'
): OmittedNames | null => {
	if (raw === undefined) return null;
	if (pass_env !== 'all') throw new TypeError("`omit_env` is only valid with `pass_env: 'all'`");
	if (!isArray(raw)) throw new TypeError('`omit_env` is an array of names');
	const names = create_record<Record<string, true>>({});
	const length = raw.length;
	for (let i = 0; i < length; i++) {
		const name = check_env_name(raw[i], 'an `omit_env` entry');
		if (name.includes('*')) {
			throw new TypeError(`an \`omit_env\` entry is an exact name, not a glob: ${name}`);
		}
		names[name] = true;
	}
	return freeze(names);
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

const copy_all_env = (env: Record<string, string>, omitted: OmittedNames | null): void => {
	const names = keys(host_env);
	for (let i = 0; i < names.length; i++) {
		const name = names[i]!;
		if (omitted === null || omitted[name] !== true) copy_host_var(env, name);
	}
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
	readonly omitted: OmittedNames | null;
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
	const omitted = read_omit_env(read_own(options, 'omit_env'), pass_env);
	const values = read_env_values(read_own(options, 'env'));
	return create_record({ pass_env, omitted, values });
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
const to_program_env = ({ pass_env, omitted, values }: ProgramOptions): Record<string, string> => {
	const env = create_record<Record<string, string>>({});
	if (pass_env === 'all') {
		copy_all_env(env, omitted);
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
	stdio: SpawnOptions['stdio'],
	group: boolean
): ChildProcess => {
	try {
		return node_spawn(
			pin,
			argv,
			create_record<SpawnOptions>({
				argv0,
				cwd,
				detached: group,
				env: create_record<Record<string, string>>({ ...env }),
				shell: false,
				stdio
			})
		);
	} catch (error) {
		throw to_launch_error(name, pin, error);
	}
};

const to_result = (
	code: number | null,
	signal: string | null,
	stop: StopReason | null
): SpawnResult => {
	if (stop !== null) {
		return freeze(create_record<SpawnResult>({ kind: stop, code, signal }));
	}
	return freeze(
		signal !== null
			? create_record<SpawnResult>({ kind: 'signaled', signal })
			: // `exit` always has a code or a signal for a process that started
				create_record<SpawnResult>({ kind: 'exited', code: code! })
	);
};

type StopReason = 'timed_out' | 'aborted';

const ignore_error = (): void => {
	// a stream error has nothing to report past the child's own result
};

/**
 * Sends `signal` to the group `pid` leads, `false` when no such group is left.
 * Only ever reached with the pid of a child this module started as a group leader.
 */
const signal_group = (pid: number, signal: NodeJS.Signals | 0): boolean => {
	try {
		process_kill(-pid, signal);
		return true;
	} catch {
		return false;
	}
};

/**
 * One child, from launch until it has ended and its pipes are done with.
 */
class ChildWatch {
	readonly name: string;
	readonly pin: string;
	readonly child: ChildProcess;
	readonly group: boolean;
	readonly feeds: Array<OutputFeed> = [];
	readonly started: Promise<void>;
	readonly closed: Promise<SpawnResult>;
	#resolve_closed!: (result: SpawnResult) => void;
	#exited = false;
	#settled = false;
	#stop: StopReason | null = null;
	#code: number | null = null;
	#signal: string | null = null;
	#timeout_timer: ReturnType<typeof set_timer> | null = null;
	#grace_timer: ReturnType<typeof set_timer> | null = null;
	#drain_timer: ReturnType<typeof set_timer> | null = null;
	#abort_listener: Disposable | null = null;
	#drain_cut = false;

	constructor(name: string, pin: string, child: ChildProcess, group: boolean) {
		this.name = name;
		this.pin = pin;
		this.child = child;
		this.group = group;
		this.closed = new Promise((resolve) => {
			this.#resolve_closed = resolve;
		});
		// the only rejection of a launch that fails, seen through `started` by every caller
		this.started = new Promise((resolve, reject) => {
			child.on('spawn', resolve);
			// also fires for a failed `kill` of a running child, which isn't a launch failure
			child.on('error', (error) => {
				if (child.pid === undefined) reject(to_launch_error(name, pin, error));
			});
		});
		this.started.catch(ignore_error);
		child.stdin?.on('error', ignore_error);
		child.stdout?.on('error', ignore_error);
		child.stderr?.on('error', ignore_error);
		child.on('exit', (code, signal) => {
			this.#on_exit(code, signal);
		});
		child.on('close', (code, signal) => {
			if (!this.#exited) {
				this.#code = code;
				this.#signal = signal;
			}
			this.#settle();
		});
	}

	get drain_cut(): boolean {
		return this.#drain_cut;
	}

	/**
	 * Arms the timeout and takes the abort listener once the child is known to be running.
	 */
	arm(timeout_ms: number | null, abort_listener: Disposable | null): void {
		if (timeout_ms !== null) {
			this.#timeout_timer = set_timer(() => {
				this.stop('timed_out');
			}, timeout_ms);
			this.#timeout_timer.unref();
		}
		this.#abort_listener = abort_listener;
	}

	/**
	 * Sends `signal` to the child or its group, while the child is running.
	 */
	send(signal: NodeJS.Signals): boolean {
		if (this.#exited) return false;
		return this.#send(signal);
	}

	#send(signal: NodeJS.Signals): boolean {
		if (this.group) return signal_group(this.child.pid!, signal);
		return this.child.kill(signal);
	}

	/**
	 * Stops the child, `SIGTERM` and then `SIGKILL`, recording why when `reason` is given.
	 */
	stop(reason: StopReason | null): void {
		if (this.#exited || this.#settled) return;
		if (this.#stop === null) this.#stop = reason;
		this.#send('SIGTERM');
		if (this.#grace_timer !== null) return;
		this.#grace_timer = set_timer(() => {
			this.#on_grace_end();
		}, STOP_GRACE_MS);
		this.#grace_timer.unref();
	}

	/**
	 * The grace is over. A group is still signaled when its leader has exited, since what
	 * ignored `SIGTERM` may have outlived it with its pipes redirected, and its tracking ends here.
	 * A group keeps its id while any member lives, so a kill can only reach a recycled id
	 * once the group has emptied, within this bound.
	 */
	#on_grace_end(): void {
		this.#grace_timer = null;
		this.#send('SIGKILL');
		if (this.#settled) untrack_child(this);
	}

	/**
	 * Kills the child or group at once and drops its pipes, for a captured call over its cap.
	 */
	kill_for_cap(): void {
		this.#send('SIGKILL');
		this.child.stdout?.destroy();
		this.child.stderr?.destroy();
	}

	/**
	 * Kills what can't be left when the host exits. A group is killed even after its
	 * leader exits, since something may still be in it. Never throws.
	 */
	kill_for_host_exit(): void {
		try {
			if (this.group) signal_group(this.child.pid!, 'SIGKILL');
			else if (!this.#exited) this.child.kill('SIGKILL');
		} catch {
			// the host is exiting, and a failed kill can't be allowed to stop that
		}
	}

	#on_exit(code: number | null, signal: string | null): void {
		this.#exited = true;
		this.#code = code;
		this.#signal = signal;
		this.#clear_timeout_timer();
		// a pid alone can't be signaled once it's reaped, a group still can
		if (!this.group && this.#grace_timer !== null) {
			clear_timer(this.#grace_timer);
			this.#grace_timer = null;
		}
		this.#drain_timer = set_timer(() => {
			this.#on_drain_bound();
		}, EXIT_DRAIN_MS);
		// not unref'd: a paused stream holds no active handle, and `closed` must still settle
	}

	/**
	 * The direct child has exited and something still holds a pipe: kill what's left of the
	 * group, stop reading, and let the call finish.
	 */
	#on_drain_bound(): void {
		this.#drain_timer = null;
		const { stdout, stderr } = this.child;
		this.#drain_cut =
			(stdout !== null && !stdout.readableEnded) || (stderr !== null && !stderr.readableEnded);
		if (this.group) signal_group(this.child.pid!, 'SIGKILL');
		for (let i = 0; i < this.feeds.length; i++) this.feeds[i]!.on_drain_bound();
		if (this.feeds.length === 0) {
			stdout?.destroy();
			stderr?.destroy();
		}
		this.#settle();
	}

	#clear_timeout_timer(): void {
		if (this.#timeout_timer !== null) clear_timer(this.#timeout_timer);
		this.#timeout_timer = null;
	}

	#settle(): void {
		if (this.#settled) return;
		this.#settled = true;
		this.#clear_timeout_timer();
		if (this.#drain_timer !== null) clear_timer(this.#drain_timer);
		dispose_abort_listener(this.#abort_listener);
		// a pending grace kill still needs the exit and signal hooks to know this group, unless
		// it's already empty
		if (this.#grace_timer !== null && this.group && !signal_group(this.child.pid!, 0)) {
			clear_timer(this.#grace_timer);
			this.#grace_timer = null;
		}
		if (this.#grace_timer === null) untrack_child(this);
		this.#resolve_closed(to_result(this.#code, this.#signal, this.#stop));
	}
}

// never exposed: what the exit hook and the signal hook walk, a child leaving once it's done
const running_children = new Set<ChildWatch>();
let exit_hook_installed = false;
let running_groups = 0;

// `SIGQUIT` is left out on purpose: a listener moves it to the event loop, and a hung host
// must still dump core, the reason to send it
const HOST_SIGNALS: ReadonlyArray<NodeJS.Signals> = freeze(['SIGINT', 'SIGTERM', 'SIGHUP']);

// the signals the hook takes only while a group runs, a child in the terminal's group getting
// these from the terminal
const GROUP_ONLY_HOST_SIGNALS: ReadonlyArray<NodeJS.Signals> = freeze(['SIGINT', 'SIGHUP']);

// names the signal hook across copies of this module, each with its own children to signal
const SIGNAL_HOOK_KEY = Symbol.for('@fuzdev/gro/spawn.signal_hook');

/**
 * Whether any listener on `signal` isn't a copy of the signal hook. Raw listeners include
 * `once` wrappers, which a handler already running has removed from the plain count.
 */
const has_other_signal_listener = (signal: NodeJS.Signals): boolean => {
	const listeners = process_raw_listeners(signal) as unknown as ReadonlyArray<
		Record<symbol, unknown>
	>;
	for (let i = 0; i < listeners.length; i++) {
		if (listeners[i]![SIGNAL_HOOK_KEY] !== true) return true;
	}
	return false;
};

/**
 * Registered for the host's `exit`. Synchronous and exception-free, so it can't hang or crash
 * the exit, covering `process.exit` and an uncaught exception alike (`invoke.ts`'s handler exits).
 */
const on_host_exit = (): void => {
	try {
		for (const watch of running_children) watch.kill_for_host_exit();
	} catch {
		// patched intrinsics at worst, and the exit must still happen
	}
};

/**
 * Registered for the signals that end the host: `SIGTERM` while any child runs, the others
 * only while a group is running, since a terminal's interrupt never reaches a child in its own
 * session. Prepended so it counts a `process.once` listener before that listener removes itself.
 * When another listener is on the signal it decides what happens, and the `exit` hook covers
 * the children if that's an exit. Alone but for copies of itself, it kills its groups, sends
 * a `SIGTERM` on to the children sharing its group, which a `SIGKILL` wouldn't let pass it to
 * their own, and raises the signal again with this listener gone, so the host dies by the
 * signal's default action, each copy signaling its own children in turn.
 */
const on_host_signal = (signal: NodeJS.Signals): void => {
	if (has_other_signal_listener(signal)) return;
	try {
		for (const watch of running_children) {
			if (watch.group) watch.kill_for_host_exit();
			else if (signal === 'SIGTERM') watch.send('SIGTERM');
		}
	} catch {
		// the signal still gets its default action below
	}
	disarm_signal_hooks();
	process_kill(host_pid, signal);
};

(on_host_signal as unknown as Record<symbol, unknown>)[SIGNAL_HOOK_KEY] = true;

const arm_group_signal_hooks = (): void => {
	for (let i = 0; i < GROUP_ONLY_HOST_SIGNALS.length; i++) {
		process_prepend_listener(GROUP_ONLY_HOST_SIGNALS[i]!, on_host_signal);
	}
};

const disarm_group_signal_hooks = (): void => {
	for (let i = 0; i < GROUP_ONLY_HOST_SIGNALS.length; i++) {
		process_off(GROUP_ONLY_HOST_SIGNALS[i]!, on_host_signal);
	}
};

const disarm_signal_hooks = (): void => {
	for (let i = 0; i < HOST_SIGNALS.length; i++) process_off(HOST_SIGNALS[i]!, on_host_signal);
};

const track_child = (watch: ChildWatch): void => {
	if (!exit_hook_installed) {
		exit_hook_installed = true;
		process_on('exit', on_host_exit);
	}
	running_children.add(watch);
	if (running_children.size === 1) process_prepend_listener('SIGTERM', on_host_signal);
	if (watch.group && running_groups++ === 0) arm_group_signal_hooks();
};

const untrack_child = (watch: ChildWatch): void => {
	if (!running_children.delete(watch)) return;
	if (watch.group && --running_groups === 0) disarm_group_signal_hooks();
	if (running_children.size === 0) process_off('SIGTERM', on_host_signal);
};

/**
 * A call's options that start a child, read once into a null-prototype object.
 */
interface CallOptions {
	readonly cwd: string;
	readonly timeout_ms: number | null;
	readonly signal: AbortSignal | null;
}

const read_timeout_ms = (raw: unknown): number | null => {
	if (raw === undefined) return null;
	if (typeof raw !== 'number' || !isSafeInteger(raw) || raw <= 0 || raw > MAX_TIMEOUT_MS) {
		throw new TypeError('`timeout_ms` is a positive safe integer of at most 2147483647');
	}
	return raw;
};

const read_signal = (raw: unknown): AbortSignal | null => {
	if (raw === undefined) return null;
	if (!(raw instanceof AbortSignalClass)) throw new TypeError('`signal` is an `AbortSignal`');
	return raw;
};

const read_call_options = (raw: Readonly<Record<string, unknown>> | null): CallOptions =>
	create_record({
		cwd: read_cwd(read_own(raw, 'cwd')),
		timeout_ms: read_timeout_ms(read_own(raw, 'timeout_ms')),
		signal: read_signal(read_own(raw, 'signal'))
	});

/**
 * Listens for `signal`'s abort, surviving `stopImmediatePropagation`.
 * @throws `TypeError` - for a forged signal, one `instanceof` passes that can't take a listener
 */
const listen_for_abort = (signal: AbortSignal, listener: () => void): Disposable => {
	try {
		return addAbortListener(signal, listener);
	} catch (error) {
		throw new TypeError('`signal` is an `AbortSignal`', { cause: error });
	}
};

/**
 * Never throws, so a forged signal's `removeEventListener` can't crash the host from an event.
 */
const dispose_abort_listener = (listener: Disposable | null): void => {
	try {
		listener?.[Symbol.dispose]();
	} catch {
		// nothing to report and nothing left to clean up
	}
};

/**
 * Refuses a call whose signal already aborted, before anything starts.
 * @throws `SpawnError` - `launch_failed`
 */
const check_not_aborted = (name: string, pin: string, signal: AbortSignal | null): void => {
	if (signal?.aborted !== true) return;
	throw new SpawnError(
		'launch_failed',
		name,
		`launch_failed: ${name} (${pin}): the signal aborted before it started`,
		{ cause: signal.reason }
	);
};

/**
 * Whether a child with these streams leads its own group. Any inherited stream keeps the child
 * in the terminal's session, where `/dev/tty` and Ctrl-C still reach it.
 */
const is_grouped = (stdio: ReadonlyArray<SpawnStdio>): boolean =>
	stdio[0] !== 'inherit' && stdio[1] !== 'inherit' && stdio[2] !== 'inherit';

const to_ref_stream = (stream: Readable): Readable & { ref?: () => void; unref?: () => void } =>
	stream;

/**
 * A chunk as it passes through promise resolution, null-prototype so it's never thenable,
 * where a bare `Uint8Array` would adopt a polluted `Object.prototype.then`.
 */
interface OutputEntry {
	readonly chunk: Uint8Array;
}

/**
 * A started child's piped output: chunks read from the stream, handed to one reader in order.
 * Reading starts at once, so nothing is lost before the reader arrives, up to the
 * buffer, past which the stream pauses for the reader to catch up.
 */
class OutputFeed {
	readonly #stream: Readable;
	readonly #chunks: Array<Uint8Array> = [];
	readonly #waiters: Array<(entry: OutputEntry | null) => void> = [];
	#head = 0;
	#buffered = 0;
	#ended = false;
	#discarding = false;
	#paused = false;
	#bounded = false;
	#claimed = false;

	constructor(stream: Readable) {
		this.#stream = stream;
		stream.on('data', (chunk: Uint8Array) => {
			this.#on_chunk(chunk);
		});
		stream.on('close', () => {
			this.#on_close();
		});
	}

	claim(): void {
		if (this.#claimed) throw new TypeError('a child output stream has one reader, once');
		this.#claimed = true;
	}

	#on_chunk(chunk: Uint8Array): void {
		if (this.#discarding) return;
		// a copy, so a reader never holds a `Buffer` or a view of memory Node reuses
		const copy = new Uint8ArrayClass(chunk);
		if (this.#waiters.length > 0) {
			this.#waiters.shift()!(create_record<OutputEntry>({ chunk: copy }));
			return;
		}
		this.#chunks[this.#chunks.length] = copy;
		this.#buffered += copy.length;
		// unconditional, as Node resumes every output stream once when the child exits, paused or not
		if (this.#buffered > OUTPUT_BUFFER_BYTES) {
			this.#paused = true;
			this.#stream.pause();
			if (this.#bounded) to_ref_stream(this.#stream).unref?.();
		}
	}

	#on_close(): void {
		this.#ended = true;
		const waiters = this.#waiters.splice(0);
		for (let i = 0; i < waiters.length; i++) waiters[i]!(null);
	}

	/**
	 * The next chunk in its entry, or `null` once the output ends.
	 */
	next(): Promise<OutputEntry | null> | OutputEntry | null {
		if (this.#head < this.#chunks.length) {
			const chunk = this.#chunks[this.#head]!;
			this.#chunks[this.#head++] = undefined as unknown as Uint8Array;
			this.#buffered -= chunk.length;
			if (this.#head === this.#chunks.length) {
				this.#chunks.length = 0;
				this.#head = 0;
			}
			this.#resume_if_room();
			return create_record<OutputEntry>({ chunk });
		}
		if (this.#ended || this.#discarding) return null;
		return new Promise((resolve) => {
			this.#waiters[this.#waiters.length] = resolve;
		});
	}

	#resume_if_room(): void {
		if (!this.#paused || this.#buffered > OUTPUT_BUFFER_BYTES / 2) return;
		this.#paused = false;
		this.#stream.resume();
		if (this.#bounded && !this.#ended) {
			// a reader's pending `next` must keep the host running until the rest arrives
			to_ref_stream(this.#stream).ref?.();
			this.#arm_destroy();
		}
	}

	/**
	 * The reader left early: drop what's held, keep the pipe flowing so the child can't block.
	 */
	discard(): void {
		this.#discarding = true;
		this.#chunks.length = 0;
		this.#head = 0;
		this.#buffered = 0;
		if (this.#paused) {
			this.#paused = false;
			this.#stream.resume();
		}
		const waiters = this.#waiters.splice(0);
		for (let i = 0; i < waiters.length; i++) waiters[i]!(null);
	}

	/**
	 * The drain bound passed. A stream nobody reads, or one that isn't behind, is finished now,
	 * and what's buffered stays readable. One paused for a reader behind it keeps its data,
	 * no longer holding the host's event loop open, and gets the bound again after it resumes.
	 */
	on_drain_bound(): void {
		if (this.#ended) return;
		if (this.#paused && this.#claimed) {
			this.#bounded = true;
			to_ref_stream(this.#stream).unref?.();
			return;
		}
		this.#stream.destroy();
	}

	#arm_destroy(): void {
		const timer = set_timer(() => {
			if (!this.#paused) this.#stream.destroy();
		}, EXIT_DRAIN_MS);
		timer.unref();
	}
}

/**
 * An output stream as a reader sees it. Holding the feed privately is what
 * keeps the stream itself out of reach.
 */
class NodeSpawnOutput implements AsyncIterable<Uint8Array> {
	readonly #feed: OutputFeed;

	constructor(key: symbol, feed: OutputFeed) {
		check_handle_key(key);
		this.#feed = feed;
	}

	[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
		this.#feed.claim();
		return freeze(new NodeSpawnOutputIterator(HANDLE_KEY, this.#feed));
	}
}

const iterator_result = (chunk: Uint8Array | null): IteratorResult<Uint8Array> =>
	chunk === null
		? create_record<IteratorResult<Uint8Array>>({ done: true, value: undefined })
		: create_record<IteratorResult<Uint8Array>>({ done: false, value: chunk });

class NodeSpawnOutputIterator implements AsyncIterator<Uint8Array> {
	readonly #feed: OutputFeed;

	constructor(key: symbol, feed: OutputFeed) {
		check_handle_key(key);
		this.#feed = feed;
	}

	async next(): Promise<IteratorResult<Uint8Array>> {
		const entry = await this.#feed.next();
		return iterator_result(entry === null ? null : entry.chunk);
	}

	// reached by leaving a `for await` early
	return(): Promise<IteratorResult<Uint8Array>> {
		this.#feed.discard();
		return Promise.resolve(iterator_result(null));
	}
}

class NodeSpawnStdin implements SpawnStdin {
	readonly #stream: Writable;

	constructor(key: symbol, stream: Writable) {
		check_handle_key(key);
		this.#stream = stream;
	}

	write(data: string | Uint8Array): Promise<boolean> {
		const stream = this.#stream;
		return new Promise((resolve, reject) => {
			if (typeof data === 'string') {
				if (!data.isWellFormed()) {
					reject(new TypeError("stdin data isn't well-formed Unicode"));
					return;
				}
			} else if (!isUint8Array(data)) {
				reject(new TypeError('stdin data is a string or a `Uint8Array`'));
				return;
			}
			// a copy, as Node holds what it can't write yet and a caller may reuse the buffer
			const chunk = typeof data === 'string' ? data : new Uint8ArrayClass(data);
			// the callback gets the error or none, and fires on a destroyed stream too
			stream.write(chunk, (error) => {
				resolve(error === null || error === undefined);
			});
		});
	}

	close(): Promise<void> {
		const stream = this.#stream;
		return new Promise((resolve) => {
			if (stream.destroyed || stream.writableFinished) {
				resolve();
				return;
			}
			stream.end(() => {
				resolve();
			});
		});
	}
}

class NodeSpawnChild implements SpawnChild {
	readonly pid: number;
	readonly closed: Promise<SpawnResult>;
	readonly stdout: AsyncIterable<Uint8Array> | null;
	readonly stderr: AsyncIterable<Uint8Array> | null;
	readonly stdin: SpawnStdin | null;
	readonly #watch: ChildWatch;

	constructor(
		key: symbol,
		watch: ChildWatch,
		stdout: OutputFeed | null,
		stderr: OutputFeed | null
	) {
		check_handle_key(key);
		this.#watch = watch;
		this.pid = watch.child.pid!;
		this.closed = freeze(watch.closed);
		this.stdout = stdout === null ? null : freeze(new NodeSpawnOutput(HANDLE_KEY, stdout));
		this.stderr = stderr === null ? null : freeze(new NodeSpawnOutput(HANDLE_KEY, stderr));
		const { stdin } = watch.child;
		this.stdin = stdin === null ? null : freeze(new NodeSpawnStdin(HANDLE_KEY, stdin));
	}

	kill(signal: SpawnSignal = 'SIGTERM'): boolean {
		const watch = this.#watch;
		return watch.send(check_signal(signal));
	}

	async [Symbol.asyncDispose](): Promise<void> {
		const watch = this.#watch;
		watch.stop(null);
		await watch.closed;
	}
}

const SIGNAL_NAMES: Readonly<Record<string, true | undefined>> = freeze(
	create_record<Record<string, true>>({
		SIGTERM: true,
		SIGKILL: true,
		SIGINT: true,
		SIGHUP: true,
		SIGQUIT: true,
		SIGUSR1: true,
		SIGUSR2: true
	})
);

const check_signal = (signal: unknown): NodeJS.Signals => {
	if (typeof signal !== 'string' || SIGNAL_NAMES[signal] !== true) {
		throw new TypeError('a signal is one of the `SpawnSignal` names');
	}
	return signal as NodeJS.Signals;
};

interface Capture {
	readonly chunks: Array<Uint8Array>;
	size: number;
}

const to_captured_text = (capture: Capture): string => {
	const bytes = new Uint8ArrayClass(capture.size);
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
): CallOptions & { max_bytes: number } => {
	const raw = check_call_options(options);
	const call = read_call_options(raw);
	const max_bytes = read_own(raw, 'max_bytes');
	if (max_bytes === undefined) return create_record({ ...call, max_bytes: DEFAULT_MAX_BYTES });
	if (typeof max_bytes !== 'number' || !isSafeInteger(max_bytes) || max_bytes < 0) {
		throw new TypeError('`max_bytes` is a non-negative safe integer');
	}
	return create_record({ ...call, max_bytes });
};

/**
 * A `stdio` option, read once: one value for all three streams, or the three in order.
 */
const read_stdio = (raw: unknown): [SpawnStdio, SpawnStdio, SpawnStdio] => {
	if (raw === undefined) return ['inherit', 'inherit', 'inherit'];
	if (typeof raw === 'string') {
		const stdio = check_stdio(raw);
		return [stdio, stdio, stdio];
	}
	if (!isArray(raw) || raw.length !== 3) {
		throw new TypeError("`stdio` is 'inherit', 'pipe', 'ignore', or three of them");
	}
	return [check_stdio(raw[0]), check_stdio(raw[1]), check_stdio(raw[2])];
};

const check_stdio = (raw: unknown): SpawnStdio => {
	if (raw !== 'inherit' && raw !== 'pipe' && raw !== 'ignore') {
		throw new TypeError("a `stdio` value is 'inherit', 'pipe', or 'ignore'");
	}
	return raw;
};

/**
 * Launches a child and watches it, its timeout and abort armed, tracked by the exit hook.
 */
const launch_watched = (
	name: string,
	argv0: string,
	pin: string,
	env: Record<string, string>,
	argv: Array<string>,
	call: CallOptions,
	stdio: SpawnOptions['stdio'],
	group: boolean
): ChildWatch => {
	check_not_aborted(name, pin, call.signal);
	// listens before launch, so a forged signal that throws here leaves no child nobody holds
	let watch: ChildWatch | null = null;
	const abort_listener =
		call.signal === null
			? null
			: listen_for_abort(call.signal, () => {
					watch?.stop('aborted');
				});
	let child: ChildProcess;
	try {
		child = launch(name, argv0, pin, env, argv, call.cwd, stdio, group);
	} catch (error) {
		dispose_abort_listener(abort_listener);
		throw error;
	}
	watch = new ChildWatch(name, pin, child, group);
	if (child.pid === undefined) {
		dispose_abort_listener(abort_listener);
	} else {
		track_child(watch);
		watch.arm(call.timeout_ms, abort_listener);
	}
	return watch;
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
		const call = read_call_options(check_call_options(options));
		const watch = launch_watched(
			this.name,
			this.#argv0,
			pin,
			this.#env,
			argv,
			call,
			'inherit',
			false
		);
		await watch.started;
		return watch.closed;
	}

	async out(args: ReadonlyArray<string>, options?: SpawnOutOptions): Promise<SpawnOutput> {
		const pin = this.#pin;
		const argv = to_argv(args);
		const call = read_out_options(options);
		const { max_bytes } = call;
		const watch = launch_watched(
			this.name,
			this.#argv0,
			pin,
			this.#env,
			argv,
			call,
			['ignore', 'pipe', 'pipe'],
			true
		);
		const { child } = watch;
		const stdout: Capture = { chunks: [], size: 0 };
		const stderr: Capture = { chunks: [], size: 0 };
		let capped = false;
		const collect = (capture: Capture) => (chunk: Uint8Array) => {
			if (capped || !add_chunk(capture, chunk, max_bytes)) return;
			capped = true;
			watch.kill_for_cap();
		};
		child.stdout?.on('data', collect(stdout));
		child.stderr?.on('data', collect(stderr));
		await watch.started;
		const result = await watch.closed;
		return freeze(
			create_record({
				...result,
				stdout: to_captured_text(stdout),
				stderr: to_captured_text(stderr),
				truncated: capped || watch.drain_cut
			})
		);
	}

	async start(args: ReadonlyArray<string>, options?: SpawnStartOptions): Promise<SpawnChild> {
		const pin = this.#pin;
		const argv = to_argv(args);
		const raw = check_call_options(options);
		const call = read_call_options(raw);
		const stdio = read_stdio(read_own(raw, 'stdio'));
		const watch = launch_watched(
			this.name,
			this.#argv0,
			pin,
			this.#env,
			argv,
			call,
			stdio,
			is_grouped(stdio)
		);
		// a launch that failed has no pid, and for `EMFILE` and `ENFILE` no streams, so this rejects
		if (watch.child.pid === undefined) await watch.started;
		const { stdout, stderr } = watch.child;
		const stdout_feed = stdout === null ? null : new OutputFeed(stdout);
		const stderr_feed = stderr === null ? null : new OutputFeed(stderr);
		if (stdout_feed !== null) watch.feeds[watch.feeds.length] = stdout_feed;
		if (stderr_feed !== null) watch.feeds[watch.feeds.length] = stderr_feed;
		await watch.started;
		return freeze(new NodeSpawnChild(HANDLE_KEY, watch, stdout_feed, stderr_feed));
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

// the child and what it holds are also null-prototype, so a polluted `Object.prototype.then` can't
// make `await child` adopt a forged value
for (const handle_class of [
	NodeSpawnChild,
	NodeSpawnStdin,
	NodeSpawnOutput,
	NodeSpawnOutputIterator
]) {
	setPrototypeOf(handle_class.prototype, null);
	freeze(handle_class.prototype);
	freeze(handle_class);
}

/**
 * The spawn host module's root handle. Import it where authority enters
 * (a task, a script, an entry module) and hand a narrower `spawn.narrow(...)`
 * or a single `SpawnProgram` down.
 */
export const spawn: Spawn = freeze(new NodeSpawn(HANDLE_KEY, null));
