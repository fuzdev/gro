/**
 * The process host module: the environment, script args, working directory, exit, and stdio
 * of the running process, as an import rather than a global.
 *
 * `process` deliberately shares a name with Node's global, so an importer's bare `process`
 * means this module, and the export shadows the global only where it's imported. The runtime has no
 * global at all, so a module that reaches the process shows it in its imports.
 *
 * An `Env` is read-only and holds a snapshot taken when this module loads, the runtime's
 * fixed environment: a write to the global's `process.env` afterward changes nothing here,
 * and only names listed by `env.narrow` reach a narrowed handle. A name outside its set throws
 * `not_granted`, so a denial is never read as a variable that's unset, which is
 * `undefined` from `get`. Library edges take plain records, built at a root from `get_all`.
 * An environment value can be a secret: this module prints none, its errors naming the variable,
 * never the value, so what a value becomes after a read is up to the caller.
 *
 * `args` are the arguments after the script, a frozen array, what a script given to `gro run` receives
 * and what a bare `node script.ts` receives. In the CLI's own process they're
 * the CLI's args, the task name included, since a task's parsed args are on its context.
 *
 * Handles and their prototypes are frozen, null-prototype so none of them is thenable even with a
 * polluted `Object.prototype.then`, and constructing one takes a key only this module holds.
 * A method called on any other object throws `TypeError`, private fields being the only state.
 * `args` is no handle but a real array, frozen, so its prototype is the unfrozen `Array.prototype`
 * and awaiting it adopts a polluted `then`.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can read `process.env` or call `process.exit`,
 *   so a narrowed `Env` bounds only code that uses it, a review signal, not a boundary.
 * - The root `Env` allows every name, where the runtime's holds only the granted ones and
 *   a name outside them throws `not_granted`.
 * - The snapshot is taken at this module's load, so a write to `process.env` before that shows.
 * - `args` under `node -e` and `node -p` reads the undocumented `process._eval`,
 *   as no documented API says the process has no script.
 * - `exit` in a Node worker ends the thread, not the process.
 * - On POSIX Node's stdio writes to files and terminals block the event loop until the kernel
 *   takes them, where the runtime's complete on its own schedule. Writes to pipes are asynchronous.
 * - Intrinsics stay pollutable. The Node functions this module calls are copied at load, but
 *   Node's stream `write` is the stream's own, so code that patched it runs inside a write, and the
 *   streams are created on first use, so one replaced on the global by then is the one written to.
 * - Importing this module throws in a removed or non-UTF-8 working directory, as `fs` does.
 *
 * @module
 */

import * as node_tty from 'node:tty';
import * as node_util from 'node:util';
import * as node_worker_threads from 'node:worker_threads';

import { to_checked_cwd } from './internal/cwd.ts';
import { ProcessError } from './process_error.ts';

// the global, under another name since this module's export shadows it
const host_process = globalThis.process;
const process_exit: (code?: number) => never = host_process.exit.bind(host_process);
const { isatty } = node_tty;
const { isUint8Array } = node_util.types;
const { isMainThread } = node_worker_threads;
const { isArray } = Array;
const { isInteger } = Number;
const { assign, create, freeze, keys, setPrototypeOf } = Object;
const Uint8ArrayClass = Uint8Array;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('process handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('process handles come from `process`, not a constructor');
	}
};

// a null-prototype record, so no name resolves through `Object.prototype`
const create_record = <T extends object>(fields: T): T => assign(create(null) as T, fields);

type Allowed = Readonly<Record<string, true | undefined>>;

/**
 * The environment, read-only, a snapshot taken when the module loads.
 */
export interface Env {
	/**
	 * A variable's value, `undefined` when it's unset. Own names only, so `toString` is unset.
	 * @throws `TypeError` - for a name that isn't a non-empty, well-formed string without NUL or `=`
	 * @throws `ProcessError` - `not_granted`, a name outside this handle's set
	 */
	get(name: string): string | undefined;
	/**
	 * Every variable this handle allows that's set, a frozen null-prototype record.
	 * Pass it, or part of it, where a library takes a plain record.
	 */
	get_all(): Readonly<Record<string, string>>;
	/**
	 * A handle limited to `names`, each an exact, case-sensitive name, never a glob, so a holder
	 * of the handle can't enumerate more than it's given. A name that's unset stays
	 * in the set, as `undefined` from `get`. It can only shrink.
	 * @throws `TypeError` - for a name `get` refuses, or one holding `*`
	 * @throws `ProcessError` - `not_granted`, a name outside this handle's set
	 */
	narrow(names: ReadonlyArray<string>): Env;
}

/**
 * The process's stdin.
 */
export interface ProcessInput extends AsyncIterable<Uint8Array> {
	/**
	 * Whether stdin is a terminal.
	 */
	readonly is_terminal: boolean;
}

/**
 * The process's stdout or stderr.
 */
export interface ProcessOutput {
	/**
	 * Whether this stream is a terminal.
	 */
	readonly is_terminal: boolean;
	/**
	 * Writes `data`, a string as UTF-8, resolving once it's accepted, so awaiting each write
	 * paces a writer against the reader. Writes not awaited queue in memory, and `exit` drops
	 * any not yet accepted, so await a write before exiting.
	 * @returns `false` when the reader has gone (`EPIPE`, a task piped to `head`), so the data
	 * went nowhere, and every later write resolves `false` too
	 * @throws `TypeError` - for data that's neither a well-formed string nor a `Uint8Array`
	 * @throws `ProcessError` - `io_error`, any other write failure, like a full disk
	 */
	write(data: string | Uint8Array): Promise<boolean>;
}

/**
 * The running process.
 */
export interface Process {
	/**
	 * The environment, a frozen handle, narrowed with `env.narrow`.
	 */
	readonly env: Env;
	/**
	 * The arguments after the script, frozen. In the CLI's own process these are the CLI's,
	 * the task name included; a task reads its parsed args from its context.
	 */
	readonly args: ReadonlyArray<string>;
	/**
	 * The working directory when the module loaded, checked as for `fs.project`.
	 */
	cwd(): string;
	/**
	 * Exits with `code`, or without one the code from `set_exit_code`, else `0`.
	 * Spawned children are cleaned up on the way out, by the spawn module's exit hook.
	 * Writes not yet accepted are dropped, so await a write before exiting.
	 * @param code - an integer from 0 to 255
	 * @throws `TypeError` - for any other value, without calling a thenable or a `toPrimitive`
	 */
	exit(code?: number): never;
	/**
	 * Sets the code the process exits with when it next ends on its own.
	 * @param code - an integer from 0 to 255
	 * @throws `TypeError` - for any other value
	 */
	set_exit_code(code: number): void;
	/**
	 * Stdin. Iterating is once only and reads bytes as plain `Uint8Array` copies, a second
	 * iteration throwing `TypeError`. Leaving a loop early stops reading.
	 */
	readonly stdin: ProcessInput;
	/**
	 * Stdout.
	 */
	readonly stdout: ProcessOutput;
	/**
	 * Stderr.
	 */
	readonly stderr: ProcessOutput;
	/**
	 * The process's id, for display only: nothing in this module takes a pid.
	 */
	readonly pid: number;
	/**
	 * The runtime's name and version, opaque, like `node/v24.14.0` here and `fgro/0.1.0` on the
	 * runtime, for display and for keying caches by the runtime that made them.
	 * Compare it for equality only; a program that branches on it is one this module can't serve.
	 */
	readonly runtime_version: string;
}

/**
 * A variable name: non-empty, well-formed, without NUL or `=`, none of which can name a variable.
 */
const check_env_name = (value: unknown, what: string): string => {
	if (typeof value !== 'string') throw new TypeError(`${what} must be a string`);
	if (value.length === 0) throw new TypeError(`${what} is empty`);
	if (value.includes('\0')) throw new TypeError(`${what} holds a NUL byte`);
	if (!value.isWellFormed()) throw new TypeError(`${what} isn't well-formed Unicode`);
	// no echo of the text, as what follows a `=` can be a value
	if (value.includes('=')) throw new TypeError(`${what} holds a \`=\``);
	return value;
};

/**
 * A narrowed set's name: exact, with no `*` a reader could take for a glob.
 */
const check_narrow_name = (value: unknown): string => {
	const name = check_env_name(value, 'a `narrow` name');
	if (name.includes('*')) {
		throw new TypeError(`a \`narrow\` name is exact, never a glob: ${name}`);
	}
	return name;
};

/**
 * The environment as a frozen null-prototype record, own properties only, so nothing inherited
 * from `Object.prototype` ever reads as a variable.
 */
const to_env_snapshot = (): Readonly<Record<string, string>> => {
	const snapshot = create_record<Record<string, string>>({});
	const names = keys(host_process.env);
	for (let i = 0; i < names.length; i++) {
		const name = names[i]!;
		const value = host_process.env[name];
		if (typeof value === 'string') snapshot[name] = value;
	}
	return freeze(snapshot);
};

class NodeEnv implements Env {
	readonly #vars: Readonly<Record<string, string>>;
	readonly #allowed: Allowed | null;

	constructor(key: symbol, vars: Readonly<Record<string, string>>, allowed: Allowed | null) {
		check_handle_key(key);
		this.#vars = vars;
		this.#allowed = allowed;
	}

	get(name: string): string | undefined {
		const vars = this.#vars;
		const allowed = this.#allowed;
		const checked = check_env_name(name, 'an environment name');
		if (allowed !== null && allowed[checked] !== true) {
			throw new ProcessError(
				'not_granted',
				checked,
				`not_granted: ${checked} isn't in this env's set`
			);
		}
		return vars[checked];
	}

	get_all(): Readonly<Record<string, string>> {
		return this.#vars;
	}

	narrow(names: ReadonlyArray<string>): Env {
		const parent_vars = this.#vars;
		const parent_allowed = this.#allowed;
		if (!isArray(names)) throw new TypeError('env.narrow takes an array of names');
		const vars = create_record<Record<string, string>>({});
		const allowed = create_record<Record<string, true>>({});
		const length = names.length;
		for (let i = 0; i < length; i++) {
			const name = check_narrow_name(names[i]);
			if (parent_allowed !== null && parent_allowed[name] !== true) {
				throw new ProcessError(
					'not_granted',
					name,
					`not_granted: ${name} isn't in this env's set, and narrow only shrinks`
				);
			}
			allowed[name] = true;
			const value = parent_vars[name];
			if (value !== undefined) vars[name] = value;
		}
		return freeze(new NodeEnv(HANDLE_KEY, freeze(vars), freeze(allowed)));
	}
}

/**
 * The arguments after the script. `node -e` and `node -p` have no script, so the first
 * argument is at `argv[1]`; a script read from stdin as `-` keeps the name at `argv[1]`.
 * A worker always has something at `argv[1]`, its script or an eval marker, whatever `_eval` says.
 * `node -e code - a` reads as the stdin case, so its `-` is dropped, a known limit.
 */
const to_script_args = (): ReadonlyArray<string> => {
	const { argv, _eval } = host_process as { argv: Array<string>; _eval?: string };
	const start = isMainThread && typeof _eval === 'string' && argv[1] !== '-' ? 1 : 2;
	const args: Array<string> = [];
	const length = argv.length;
	for (let i = start; i < length; i++) args[args.length] = argv[i]!;
	return freeze(args);
};

const check_exit_code = (code: unknown): number => {
	// `typeof` first, so a thenable or `Symbol.toPrimitive` code runs no code of the caller's
	if (typeof code !== 'number' || !isInteger(code) || code < 0 || code > 255) {
		throw new TypeError('an exit code is an integer from 0 to 255');
	}
	return code;
};

const to_data_chunk = (data: unknown): string | Uint8Array => {
	if (typeof data === 'string') {
		if (!data.isWellFormed()) throw new TypeError("stdio data isn't well-formed Unicode");
		return data;
	}
	if (!isUint8Array(data)) throw new TypeError('stdio data is a string or a `Uint8Array`');
	// a copy, as Node holds what it can't write yet and a caller may reuse the buffer
	return new Uint8ArrayClass(data);
};

const ignore_error = (): void => {
	// each write's callback reports its own failure, so its `'error'` event has nothing to add
};

/**
 * Whether a write failed because the reader went away, the case a `false` result is for.
 * Node leaves `process.stdout` undestroyed after `EPIPE`, every later write erroring the same way.
 */
const is_reader_gone = (error: Error): boolean => {
	const { code } = error as { code?: unknown };
	return (
		code === 'EPIPE' ||
		code === 'ECONNRESET' ||
		code === 'ERR_STREAM_DESTROYED' ||
		code === 'ERR_STREAM_WRITE_AFTER_END'
	);
};

class NodeProcessOutput implements ProcessOutput {
	readonly #target: 'stdout' | 'stderr';
	readonly #fd: number;
	#stream: NodeJS.WriteStream | null = null;

	constructor(key: symbol, target: 'stdout' | 'stderr', fd: number) {
		check_handle_key(key);
		this.#target = target;
		this.#fd = fd;
	}

	get is_terminal(): boolean {
		return isatty(this.#fd);
	}

	/**
	 * The Node stream, made on first use since reading the global creates it.
	 */
	#writable(): NodeJS.WriteStream {
		return (this.#stream ??= host_process[this.#target]);
	}

	write(data: string | Uint8Array): Promise<boolean> {
		const target = this.#target;
		return new Promise((resolve, reject) => {
			const chunk = to_data_chunk(data);
			const stream = this.#writable();
			stream.write(chunk, (error) => {
				if (error === null || error === undefined) {
					resolve(true);
					return;
				}
				// the failure is also emitted as `'error'`, which throws with no listener; a one-shot
				// listener only for this one, as Node's `Console` does, so another writer's still throws
				if (stream.listenerCount('error') === 0) stream.once('error', ignore_error);
				if (is_reader_gone(error)) {
					resolve(false);
				} else {
					reject(
						new ProcessError('io_error', target, `io_error: ${target}: ${error.message}`, {
							cause: error
						})
					);
				}
			});
		});
	}
}

const iterator_result = (chunk: Uint8Array | null): IteratorResult<Uint8Array> =>
	chunk === null
		? create_record<IteratorResult<Uint8Array>>({ done: true, value: undefined })
		: create_record<IteratorResult<Uint8Array>>({ done: false, value: chunk });

class NodeProcessInput implements ProcessInput {
	readonly #fd = 0;
	#claimed = false;

	constructor(key: symbol) {
		check_handle_key(key);
	}

	get is_terminal(): boolean {
		return isatty(this.#fd);
	}

	[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
		if (this.#claimed) throw new TypeError('stdin has one reader, once');
		this.#claimed = true;
		return freeze(new NodeProcessInputIterator(HANDLE_KEY, host_process.stdin));
	}
}

class NodeProcessInputIterator implements AsyncIterator<Uint8Array> {
	readonly #stream: NodeJS.ReadStream;
	readonly #inner: AsyncIterator<Uint8Array>;

	constructor(key: symbol, stream: NodeJS.ReadStream) {
		check_handle_key(key);
		this.#stream = stream;
		// not destroying, as closing fd 0 would take stdin from any child started after
		this.#inner = stream.iterator({ destroyOnReturn: false });
	}

	async next(): Promise<IteratorResult<Uint8Array>> {
		// read before the `try`, so a borrowed receiver is a `TypeError`, not a stdin failure
		const inner = this.#inner;
		let result: IteratorResult<Uint8Array>;
		try {
			result = await inner.next();
		} catch (error) {
			throw new ProcessError('io_error', 'stdin', `io_error: stdin: ${(error as Error).message}`, {
				cause: error
			});
		}
		if (result.done) return iterator_result(null);
		const { value } = result;
		// not bytes if something set an encoding on the stream or forged the result through `then`
		if (!isUint8Array(value)) {
			await this.#stop();
			throw new ProcessError('io_error', 'stdin', "io_error: stdin: a chunk isn't bytes");
		}
		// a copy, so a reader never holds a `Buffer` or a view of memory Node reuses
		return iterator_result(new Uint8ArrayClass(value));
	}

	// reached by leaving a `for await` early
	async return(): Promise<IteratorResult<Uint8Array>> {
		await this.#stop();
		return iterator_result(null);
	}

	/**
	 * Stops the read, so a child inheriting stdin gets what's left and the host can exit.
	 */
	async #stop(): Promise<void> {
		const stream = this.#stream;
		await this.#inner.return?.();
		stream.pause();
		// Node's stdin stops reading on `'pause'`, which `pause` doesn't emit here since the iterator's
		// `'readable'` listener left the stream not flowing; emitted now, before the loop can poll again
		stream.emit('pause');
	}
}

class NodeProcess implements Process {
	readonly env: Env;
	readonly args: ReadonlyArray<string>;
	readonly stdin: ProcessInput;
	readonly stdout: ProcessOutput;
	readonly stderr: ProcessOutput;
	readonly pid: number;
	readonly runtime_version: string;
	readonly #cwd: string;

	constructor(key: symbol) {
		check_handle_key(key);
		this.#cwd = to_checked_cwd();
		this.env = freeze(new NodeEnv(HANDLE_KEY, to_env_snapshot(), null));
		this.args = to_script_args();
		this.stdin = freeze(new NodeProcessInput(HANDLE_KEY));
		this.stdout = freeze(new NodeProcessOutput(HANDLE_KEY, 'stdout', 1));
		this.stderr = freeze(new NodeProcessOutput(HANDLE_KEY, 'stderr', 2));
		this.pid = host_process.pid;
		this.runtime_version = `node/${host_process.version}`;
	}

	cwd(): string {
		return this.#cwd;
	}

	#check_receiver(): void {
		// a private method call, so any other receiver is a `TypeError`
	}

	exit(code?: number): never {
		this.#check_receiver();
		if (code === undefined) process_exit();
		process_exit(check_exit_code(code));
	}

	set_exit_code(code: number): void {
		this.#check_receiver();
		host_process.exitCode = check_exit_code(code);
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
for (const handle_class of [
	NodeEnv,
	NodeProcessOutput,
	NodeProcessInput,
	NodeProcessInputIterator,
	NodeProcess
]) {
	setPrototypeOf(handle_class.prototype, null);
	freeze(handle_class.prototype);
	freeze(handle_class);
}

/**
 * The process host module's root handle. Import it where authority enters (a task, a script,
 * an entry module) and hand a narrower `process.env.narrow(...)` or plain records down.
 */
export const process: Process = freeze(new NodeProcess(HANDLE_KEY));
