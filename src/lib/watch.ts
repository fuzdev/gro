/**
 * The watch host module: changes beneath an fs handle, as an async iterator of events.
 *
 * `watch.dir(dir, options?)` watches the directory a handle is on and resolves once the
 * watch is in place, reporting no entry that was already there. Each event's `path` is
 * relative to the handle and `/`-separated, the handle's own directory `''`, so a path from
 * an event goes straight back to the handle's ops. The handle is the grant: a read-only one
 * is enough, and nothing beyond it is watched or reported.
 *
 * Events are `add`, `update`, and `delete`, each with `is_directory`, and `rescan`, which
 * says changes beneath a path may have been missed and its state is to be read again:
 * the events not yet read overflowed `max_pending`, or a directory couldn't be watched.
 * They're hints for a caller to act on, not a log to replay:
 *
 * - A symlink is an entry like a file, `is_directory: false`, never followed, as `fs_search`
 *   treats one: what it points at is never watched, so a change there reports nothing,
 *   and replacing the link reports the link itself.
 * - A rename is a `delete` of the old path and an `add` of the new, unpaired,
 *   and a file replaced by a rename over it (an atomic save) may be an `update` or a
 *   `delete` then an `add`.
 * - Events for one directory arrive in the order they happened; across directories there's no order.
 * - A directory created or moved in is an `add`, then an `add` for each entry found in it
 *   once its own watch is in place. An entry created and removed before then is never seen,
 *   and one created as the watch lands may be reported twice.
 * - A directory removed or moved out is a `delete`, and its entries may or may not have
 *   `delete`s of their own.
 * - `update` is a change to a non-directory's contents or metadata; a directory's own
 *   changes show as its entries' events.
 * - Debouncing is the caller's.
 *
 * Events not yet read are buffered up to `max_pending`, by default 4096. Past it they're
 * dropped for one `rescan` of `''`, as a kernel's event queue overflows, and the events
 * after it are dropped until that `rescan` is read, since reading the tree then sees them.
 *
 * A watcher ends when it's closed (`close()`, `await using`, or leaving its `for await`),
 * when its `signal` aborts, or when the watched directory is removed, moved, or replaced.
 * Closing or aborting ends the iteration and drops the events not yet read; a watched
 * directory that's gone delivers what was queued, then the iteration throws `FsError`
 * `not_found`. An open watcher keeps the process alive, as a listening server does, and
 * once it has ended nothing of it does.
 *
 * Watchers and their iterators are frozen and null-prototype, and hold their state in
 * private fields, so a method called on any other object throws `TypeError`.
 * Events and iterator results are frozen null-prototype records,
 * so a polluted `Object.prototype.then` is never consulted on the way to the reader.
 *
 * This is the Node implementation, which runs on the Node Gro, over chokidar's
 * per-directory native backend, symlinks never followed. Its gaps against the runtime's
 * builtin, which it documents rather than the API weakening to match:
 *
 * - The handle is read for its `path`, and the watch is on that path, so a forged handle is
 *   any object with an absolute `path` whose `list` resolves.
 *   The runtime's builtin watches the directory the handle holds by descriptor.
 * - The watch is bound to the directory's path, not its inode, as an fs handle is, so a moved
 *   watched directory ends the watch as a removed one does, where the runtime's keeps
 *   watching the directory it opened.
 * - Chokidar checks an entry with `lstat` and then watches and reads it by path, so a directory
 *   swapped for a symlink in between is watched through the link. The watcher checks each path
 *   before chokidar descends into it and before reporting it, the root still the directory the
 *   watch started on and each directory on the way still a directory, a check then a use as fs's
 *   walk is: a swap between the two escapes, and chokidar can be left watching the directory
 *   a swapped-in link points at, never beneath it, and the directories beneath one renamed away
 *   for the link, their events dropped.
 *   The runtime's builtin watches by descriptor, so no path is resolved after the scan.
 * - The root is told apart by device and inode, which a filesystem can reuse: on ext4 a watched
 *   directory removed and another made at once at its path usually gets its inode, and the watch
 *   then goes on in the new one, the directory the handle's path names, before the watch is in
 *   place or after. The runtime's builtin ends on the removal itself.
 * - An entry replaced at once by one of another type can go unreported, with no `rescan`:
 *   a file replaced by a directory reports nothing, not even the directory's entries later,
 *   and a directory replaced by a symlink is an `update` of the link.
 * - Names are decoded as UTF-8 by Node, so a name that isn't UTF-8 may go unreported, or arrive
 *   with `U+FFFD` in place of its bytes, naming no entry the handle reaches, and listing its
 *   directory through the handle fails with `io_error`.
 * - Node exposes no kernel overflow, so a `rescan` comes only from `max_pending` and a
 *   directory chokidar couldn't watch, never from events the kernel dropped. A directory that
 *   can't be read when the watch starts stays unwatched after it's readable.
 * - Any module can import `node:fs` and call `watch` on any path, so a handle bounds only
 *   code that uses it, a review signal, not a boundary.
 *
 * @module
 */

import * as node_events from 'node:events';
import * as node_fs from 'node:fs';
import type { BigIntStats } from 'node:fs';
import type { FSWatcher } from 'chokidar';

import type { FsReadDir } from './fs.ts';
import { FsError } from './fs_error.ts';
import { create_record, read_own } from './internal/records.ts';

// a listener that `stopImmediatePropagation` can't reach, which a plain `addEventListener` isn't
const { addAbortListener } = node_events as unknown as {
	addAbortListener: (signal: AbortSignal, listener: () => void) => Disposable;
};
const { lstatSync } = node_fs;
const { AbortSignal: AbortSignalClass } = globalThis;
const { isSafeInteger } = Number;
const { freeze, setPrototypeOf } = Object;

/**
 * A change to an entry beneath a watched directory.
 */
export interface WatchChange {
	/**
	 * `add` for an entry that appeared, `update` for a non-directory whose contents or
	 * metadata changed, `delete` for one that's gone.
	 */
	readonly kind: 'add' | 'update' | 'delete';
	/**
	 * The entry's path relative to the watched handle, `/`-separated.
	 */
	readonly path: string;
	/**
	 * Whether the entry is a directory; a symlink is not, whatever it points at.
	 */
	readonly is_directory: boolean;
}

/**
 * Changes beneath `path` may have been missed, so its state is to be read again.
 */
export interface WatchRescan {
	readonly kind: 'rescan';
	/**
	 * The directory to read again, relative to the watched handle, `''` for all of it.
	 */
	readonly path: string;
}

/**
 * What a `Watcher` yields.
 */
export type WatchEvent = WatchChange | WatchRescan;

/**
 * The options of `watch.dir`.
 */
export interface WatchDirOptions {
	/**
	 * Whether the directories beneath are watched too, defaults to `false`,
	 * which reports the directory's own entries, a subdirectory's `add` and `delete` included.
	 */
	recursive?: boolean;
	/**
	 * Ends the watch when it aborts. Before `watch.dir` resolves, the call rejects
	 * with its reason; after, the iteration ends.
	 */
	signal?: AbortSignal;
	/**
	 * How many events not yet read are buffered before they're dropped for one `rescan` of `''`,
	 * a positive safe integer, defaults to 4096.
	 */
	max_pending?: number;
}

/**
 * A running watch: an async iterable of its events, read once.
 */
export interface Watcher extends AsyncIterable<WatchEvent>, AsyncDisposable {
	/**
	 * Ends the watch, dropping the events not yet read. Resolves once nothing of it is left running.
	 */
	close(): Promise<void>;
}

/**
 * The watch host module's root.
 */
export interface Watch {
	/**
	 * Watches the directory a handle is on, resolving once the watch is in place,
	 * with no event for what was already there.
	 * @param dir - the handle to watch, read-only or not
	 * @throws `FsError` - `not_found`, `not_a_directory`, `permission_denied`, `outside_root`, or
	 * `io_error`, as listing the handle's directory throws, and for a directory removed or replaced
	 * before the watch is in place
	 * @throws `TypeError` - for a handle without an absolute `path`, or an invalid option
	 * @throws the `signal`'s reason, when it aborts before the watch is in place
	 */
	dir(dir: FsReadDir, options?: WatchDirOptions): Promise<Watcher>;
}

// the key only this module holds, so a watcher's `constructor` can't make another
const WATCHER_KEY = Symbol('watcher');

const check_watcher_key = (key: symbol): void => {
	if (key !== WATCHER_KEY) throw new TypeError('watchers come from `watch.dir`, not a constructor');
};

const DEFAULT_MAX_PENDING = 4096;

const to_change = (kind: WatchChange['kind'], path: string, is_directory: boolean): WatchChange =>
	freeze(create_record<WatchChange>({ kind, path, is_directory }));

const to_rescan = (path: string): WatchRescan =>
	freeze(create_record<WatchRescan>({ kind: 'rescan', path }));

const DONE: IteratorResult<WatchEvent> = freeze(
	create_record<IteratorResult<WatchEvent>>({ done: true, value: undefined })
);

const to_result = (value: WatchEvent): IteratorResult<WatchEvent> =>
	freeze(create_record<IteratorResult<WatchEvent>>({ done: false, value }));

/**
 * `lstat`s a path, never throwing.
 * @returns the stats, `undefined` for a missing entry, or `null` for any other failure
 */
const lstat_quietly = (path: string): BigIntStats | undefined | null => {
	try {
		return lstatSync(path, { bigint: true, throwIfNoEntry: false });
	} catch {
		return null;
	}
};

/**
 * Reads the identity of the directory a watch starts on, so another directory or a symlink
 * at its path later is told apart.
 * @throws `FsError` - `not_found`, `not_a_directory`, `outside_root` for a symlink, or `io_error`
 */
const read_root_stats = (root: string): BigIntStats => {
	const stats = lstat_quietly(root);
	if (stats === undefined) throw new FsError('not_found', root);
	if (stats === null) throw new FsError('io_error', root, `io_error: can't lstat ${root}`);
	if (stats.isSymbolicLink()) {
		throw new FsError('outside_root', root, `outside_root: the watched ${root} is a symlink`);
	}
	if (!stats.isDirectory()) throw new FsError('not_a_directory', root);
	return stats;
};

/**
 * Reads a handle once, for its path. A forged object with an absolute `path` passes,
 * the documented gap.
 */
const read_dir_path = (dir: unknown): string => {
	if (typeof dir !== 'object' || dir === null) {
		throw new TypeError('`watch.dir` takes an `fs` directory handle');
	}
	const path: unknown = (dir as { path?: unknown }).path;
	if (
		typeof path !== 'string' ||
		!path.startsWith('/') ||
		path.includes('\0') ||
		!path.isWellFormed()
	) {
		throw new TypeError('a watched handle has an absolute `path`');
	}
	return path;
};

const read_options = (
	raw: unknown
): { recursive: boolean; signal: AbortSignal | null; max_pending: number } => {
	if (raw !== undefined && (typeof raw !== 'object' || raw === null)) {
		throw new TypeError('`watch.dir` options are an object');
	}
	const options = raw;
	const recursive = read_own(options, 'recursive') ?? false;
	if (typeof recursive !== 'boolean') throw new TypeError('`recursive` is a boolean');
	const signal = read_own(options, 'signal');
	if (signal !== undefined && !(signal instanceof AbortSignalClass)) {
		throw new TypeError('`signal` is an `AbortSignal`');
	}
	const max_pending = read_own(options, 'max_pending') ?? DEFAULT_MAX_PENDING;
	if (typeof max_pending !== 'number' || !isSafeInteger(max_pending) || max_pending < 1) {
		throw new TypeError('`max_pending` is a positive safe integer');
	}
	return { recursive, signal: signal ?? null, max_pending };
};

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

const dispose_quietly = (disposable: Disposable | null): void => {
	try {
		disposable?.[Symbol.dispose]();
	} catch {
		// a forged signal's `removeEventListener` has nothing to report to
	}
};

interface Waiter {
	resolve: (result: IteratorResult<WatchEvent>) => void;
	reject: (error: unknown) => void;
}

/**
 * The buffer between chokidar's listeners and the reader, and how the watch ended.
 */
class WatchQueue {
	readonly #max_pending: number;
	#events: Array<WatchEvent> = [];
	// a `rescan` of `''` stands for every event dropped since it was queued
	#overflowed = false;
	#waiters: Array<Waiter> = [];
	#ended = false;
	// the error the iteration throws once the queue is read, thrown once
	#failure: FsError | null = null;

	constructor(max_pending: number) {
		this.#max_pending = max_pending;
	}

	get ended(): boolean {
		return this.#ended;
	}

	push(event: WatchEvent): void {
		if (this.#ended) return;
		const waiter = this.#waiters.shift();
		if (waiter !== undefined) {
			waiter.resolve(to_result(event));
			return;
		}
		if (this.#overflowed) return;
		if (this.#events.length >= this.#max_pending) {
			this.#events = [to_rescan('')];
			this.#overflowed = true;
			return;
		}
		this.#events.push(event);
	}

	next(): Promise<IteratorResult<WatchEvent>> {
		const event = this.#events.shift();
		if (event !== undefined) {
			if (this.#events.length === 0) this.#overflowed = false;
			return Promise.resolve(to_result(event));
		}
		if (this.#ended) return this.#settle_ended();
		return new Promise((resolve, reject) => {
			this.#waiters.push({ resolve, reject });
		});
	}

	/**
	 * Ends the queue. With `failure` the queued events are still read, then it's thrown;
	 * without, they're dropped.
	 */
	end(failure?: FsError): void {
		if (this.#ended) return;
		this.#ended = true;
		if (failure === undefined) {
			this.#events = [];
		} else {
			this.#failure = failure;
		}
		// waiters exist only while the queue is empty
		const waiters = this.#waiters;
		this.#waiters = [];
		for (const waiter of waiters) {
			this.#settle_ended().then(waiter.resolve, waiter.reject);
		}
	}

	#settle_ended(): Promise<IteratorResult<WatchEvent>> {
		const failure = this.#failure;
		if (failure === null) return Promise.resolve(DONE);
		this.#failure = null;
		return Promise.reject(failure);
	}
}

// set in `NodeWatcher`'s static block, where its private fields are in scope
let to_queue: (watcher: NodeWatcher) => WatchQueue;

class WatchIterator implements AsyncIterator<WatchEvent> {
	readonly #watcher: NodeWatcher;

	constructor(key: symbol, watcher: NodeWatcher) {
		check_watcher_key(key);
		this.#watcher = watcher;
	}

	next(): Promise<IteratorResult<WatchEvent>> {
		return to_queue(this.#watcher).next();
	}

	async return(): Promise<IteratorResult<WatchEvent>> {
		await this.#watcher.close();
		return DONE;
	}
}

class NodeWatcher implements Watcher {
	readonly #queue: WatchQueue;
	readonly #stop: () => Promise<void>;
	#iterated = false;
	#closing: Promise<void> | null = null;

	static {
		to_queue = (watcher) => watcher.#queue;
	}

	constructor(key: symbol, queue: WatchQueue, stop: () => Promise<void>) {
		check_watcher_key(key);
		this.#queue = queue;
		this.#stop = stop;
	}

	[Symbol.asyncIterator](): AsyncIterator<WatchEvent> {
		if (this.#iterated) throw new TypeError('a watcher is read once');
		this.#iterated = true;
		return freeze(new WatchIterator(WATCHER_KEY, this));
	}

	close(): Promise<void> {
		this.#queue.end();
		return (this.#closing ??= this.#stop());
	}

	[Symbol.asyncDispose](): Promise<void> {
		return this.close();
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
for (const watcher_class of [WatchIterator, NodeWatcher]) {
	setPrototypeOf(watcher_class.prototype, null);
	freeze(watcher_class.prototype);
	freeze(watcher_class);
}

/**
 * Lists the directory through the handle, so a path it couldn't read is refused before anything watches it.
 */
const check_listable = async (dir: FsReadDir): Promise<void> => {
	// eslint-disable-next-line no-unreachable-loop -- one entry, or none, shows the listing opens
	for await (const _entry of dir.list('')) break;
};

const watch_dir = async (dir: FsReadDir, raw_options?: WatchDirOptions): Promise<Watcher> => {
	const root = read_dir_path(dir);
	const { recursive, signal, max_pending } = read_options(raw_options);
	// the reason is the caller's, any value
	if (signal?.aborted) throw signal.reason;
	await check_listable(dir);
	if (signal?.aborted) throw signal.reason;
	const root_stats = read_root_stats(root);

	const prefix = root === '/' ? '/' : root + '/';
	// a path beneath the root relative to it, `''` for the root, and `null` for any other
	const to_relative = (path: string): string | null =>
		path === root ? '' : path.startsWith(prefix) ? path.slice(prefix.length) : null;

	// Chokidar decides on an `lstat` and then watches and reads by path, which the kernel resolves
	// through a symlink swapped in between, so every path it reports or descends into is checked
	// against the tree as it is now: the root still the directory the watch started on, and each
	// directory between it and the entry still a directory, not a symlink. A check then a use,
	// as fs's walk is, so a swap between the two still escapes, the Node gap the docs state.

	// whether the root's path still names the directory the watch started on; a missing root passes
	// with `missing_ok`, since a removed root's last events arrive before its own removal ends the watch
	const is_root_kept = (missing_ok: boolean): boolean => {
		const stats = lstat_quietly(root);
		if (stats === undefined) return missing_ok;
		return (
			stats !== null &&
			stats.isDirectory() &&
			stats.ino === root_stats.ino &&
			stats.dev === root_stats.dev
		);
	};
	// whether each directory between the root and an entry is still a directory; one missing fails
	// too, since a removed directory's entries need no events of their own
	const passes_through_directories = (relative: string): boolean => {
		for (let i = relative.indexOf('/'); i !== -1; i = relative.indexOf('/', i + 1)) {
			if (!lstat_quietly(prefix + relative.slice(0, i))?.isDirectory()) return false;
		}
		return true;
	};
	// the nearest directory at or above an error's path that `lstat` reaches, `''` for the root,
	// since chokidar can name an entry beneath the directory it couldn't read
	const to_rescan_path = (path: unknown): string => {
		let relative = typeof path === 'string' ? to_relative(path) : null;
		while (relative) {
			if (lstat_quietly(prefix + relative)?.isDirectory()) return relative;
			const slash = relative.lastIndexOf('/');
			relative = slash === -1 ? '' : relative.slice(0, slash);
		}
		return '';
	};

	const queue = new WatchQueue(max_pending);
	let abort_listener: Disposable | null = null;
	let stopping: Promise<void> | null = null;
	// reaches `watcher` only once it exists: chokidar's only synchronous call into `ignored`,
	// as it's created, is for the root, which returns before anything stops
	const stop = (): Promise<void> =>
		(stopping ??= (async () => {
			dispose_quietly(abort_listener);
			await watcher.close();
		})());
	// rejects `watch.dir` while it waits for chokidar's scan, which a root gone meanwhile never ends
	let reject_start: ((error: FsError) => void) | null = null;
	const to_root_gone_error = (): FsError =>
		new FsError('not_found', root, `not_found: the watched ${root} is gone`);
	const end_root_gone = (): void => {
		const failure = to_root_gone_error();
		queue.end(failure);
		reject_start?.(failure);
		void stop();
	};
	// a directory chokidar would descend into, refused before it watches or reads it;
	// a non-directory's report is checked when it arrives, so the scan `lstat`s no file
	const ignored = (path: string, stats?: { isDirectory: () => boolean }): boolean => {
		if (stats !== undefined && !stats.isDirectory()) return false;
		const relative = to_relative(path);
		if (!relative) return false;
		if (!is_root_kept(true)) {
			end_root_gone();
			return true;
		}
		return !passes_through_directories(relative);
	};

	const { watch: chokidar_watch } = await import('chokidar');
	const watcher: FSWatcher = chokidar_watch(root, {
		ignoreInitial: true,
		followSymlinks: false,
		backend: 'native',
		// the raw events, a delete never held back to pair with an add
		atomic: false,
		ignored,
		...(recursive ? null : { depth: 0 })
	});

	const on_change =
		(kind: WatchChange['kind'], is_directory: boolean) =>
		(path: string): void => {
			const relative = to_relative(path);
			if (relative === null) return;
			if (relative === '') {
				// only the root's own removal names the root
				if (kind === 'delete') end_root_gone();
				return;
			}
			if (!is_root_kept(true)) {
				// another directory or a symlink at the root's path ends the watch, as its removal does
				end_root_gone();
				return;
			}
			if (!passes_through_directories(relative)) return;
			queue.push(to_change(kind, relative, is_directory));
		};
	watcher.on('add', on_change('add', false));
	watcher.on('addDir', on_change('add', true));
	// chokidar reports `change` for non-directories only
	watcher.on('change', on_change('update', false));
	watcher.on('unlink', on_change('delete', false));
	watcher.on('unlinkDir', on_change('delete', true));
	watcher.on('error', (error: unknown) => {
		// a directory chokidar couldn't watch or scan, like `EACCES` or the system's watch limit,
		// all of it when the error names no path beneath the root
		queue.push(to_rescan(to_rescan_path((error as { path?: unknown } | null)?.path)));
	});

	try {
		await new Promise<void>((resolve, reject) => {
			watcher.once('ready', resolve);
			reject_start = reject;
			if (signal !== null) {
				abort_listener = listen_for_abort(signal, () => {
					// eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- a signal's reason is the caller's, any value
					reject(signal.reason);
				});
			}
		});
		// a root gone before chokidar watched it leaves chokidar waiting for it on its parent,
		// with no event to end the watch, so the root must still be the one it started on
		if (!is_root_kept(false)) throw to_root_gone_error();
	} catch (error) {
		queue.end();
		await stop();
		throw error;
	}
	reject_start = null;

	const handle = freeze(new NodeWatcher(WATCHER_KEY, queue, stop));
	if (signal !== null) {
		dispose_quietly(abort_listener);
		abort_listener = listen_for_abort(signal, () => void handle.close());
	}
	return handle;
};

/**
 * The watch host module's root. Import it where authority enters (a task, a script,
 * an entry module), with the fs handle it watches.
 */
export const watch: Watch = freeze(create_record<Watch>({ dir: watch_dir }));
