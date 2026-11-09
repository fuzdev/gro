import type { SpawnChild, SpawnProgram, SpawnResult, SpawnStartOptions } from './spawn.ts';

/**
 * A long-lived child that can be restarted, its program and arguments kept.
 */
export interface SpawnRestartable {
	/**
	 * Stops the running child, if any, and starts a new one from the same program,
	 * so a restart reuses the pinned executable and its environment.
	 * `restart` and `stop` run in the order they're called, and a call shares the last one still
	 * pending when that's of its own kind, so the last call decides whether a child ends up running.
	 * @throws `SpawnError` - `launch_failed`, as `SpawnProgram.start` does
	 */
	restart: () => Promise<void>;
	/**
	 * Stops the running child and waits for it, `SIGTERM` and then `SIGKILL` after a grace period.
	 * A later `restart` starts a child again.
	 */
	stop: () => Promise<void>;
	/**
	 * Whether a child is being managed, following the last `restart` or `stop` requested,
	 * so it's `false` from the moment `stop` is called.
	 * A child that exited on its own stays managed until `restart` or `stop`,
	 * so a loop can read `closed` and decide. A failed `restart` doesn't change it,
	 * so it can be `true` with no `child`.
	 */
	readonly active: boolean;
	/**
	 * The current child, possibly already exited, `null` after `stop`, during a restart, and
	 * after a failed restart.
	 */
	readonly child: SpawnChild | null;
	/**
	 * How the most recent child ended. Never rejects.
	 * During a restart, and after a failed one, it's the previous child's.
	 * Read it again after a `restart` to wait on the new child.
	 */
	readonly closed: Promise<SpawnResult>;
}

/**
 * The last requested `restart` or `stop`, until it settles.
 */
interface PendingOperation {
	readonly kind: 'restart' | 'stop';
	readonly done: Promise<void>;
	/** resolves after `done` settles, its rejection included */
	readonly settled: Promise<void>;
}

/**
 * A copy of start options a caller's later changes can't reach. `cwd` and `signal` stay the
 * same handle and signal, since a handle is immutable and a signal's abort must still arrive.
 */
const copy_start_options = (options: SpawnStartOptions): SpawnStartOptions => {
	const copy = { ...options };
	if (typeof copy.stdio === 'object') copy.stdio = Object.freeze([...copy.stdio] as const);
	return Object.freeze(copy);
};

/**
 * Starts a child that can be restarted, resolving once the first child is running.
 * Library code over a program, so the caller decides the program and its environment.
 * `args` and `options` are copied once, here, so every start uses them as they were at this
 * call, and a caller changing its array or options object afterward has no effect.
 * @param program - the executable, its pin and environment fixed for every restart
 * @param args - the arguments after `argv[0]`, copied now
 * @param options - the options of every `start`, `stdio` included, stdio inherited by default,
 *   copied now, with `signal` kept so an abort still stops a child
 * @returns the restartable child
 * @throws `SpawnError` - `launch_failed`, as `SpawnProgram.start` does
 */
export const spawn_restartable = async (
	program: SpawnProgram,
	args: ReadonlyArray<string> = [],
	options?: SpawnStartOptions
): Promise<SpawnRestartable> => {
	const start_args = Object.freeze([...args]);
	const start_options = options && copy_start_options(options);
	let child: SpawnChild | null = await program.start(start_args, start_options);
	let closed = child.closed;
	let pending: PendingOperation | null = null;
	let managed = true;

	const stop_current = async (): Promise<void> => {
		if (child === null) return;
		const current = child;
		child = null;
		await current[Symbol.asyncDispose]();
	};

	const do_restart = async (): Promise<void> => {
		await stop_current();
		child = await program.start(start_args, start_options);
		closed = child.closed;
		managed = true;
	};

	const do_stop = async (): Promise<void> => {
		// before the wait, so a loop woken by this child's `closed` already reads `active` false
		managed = false;
		await stop_current();
	};

	/**
	 * Runs `operation` after whatever is pending, or shares the last pending one of the same
	 * kind, so a `stop` queued behind a `restart` is never the earlier `stop`.
	 */
	const enqueue = (
		kind: PendingOperation['kind'],
		operation: () => Promise<void>
	): Promise<void> => {
		if (pending?.kind === kind) return pending.done;
		const done = pending === null ? operation() : pending.settled.then(operation);
		const clear = (): void => {
			if (pending === current) pending = null;
		};
		const current: PendingOperation = { kind, done, settled: done.then(clear, clear) };
		pending = current;
		return done;
	};

	const restart = (): Promise<void> => enqueue('restart', do_restart);

	const stop = (): Promise<void> => enqueue('stop', do_stop);

	return {
		restart,
		stop,
		get active() {
			return pending === null ? managed : pending.kind === 'restart';
		},
		get child() {
			return child;
		},
		get closed() {
			return closed;
		}
	};
};
