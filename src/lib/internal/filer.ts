import { EMPTY_OBJECT } from '@fuzdev/fuz_util/object.ts';
import { readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import type { OmitStrict } from '@fuzdev/fuz_util/types.ts';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { UnreachableError } from '@fuzdev/fuz_util/error.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { FileFilter, PathId } from '@fuzdev/fuz_util/path.ts';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import {
	watch_dir,
	type WatchNodeFs,
	type WatcherChange,
	type WatchDirOptions,
	type WatcherChangeCallback
} from './watch_dir.ts';
import { paths } from '../paths.ts';
import { parse_imports } from './parse_imports.ts';
import { is_path_specifier, to_specifier_path_id } from '../resolve_specifier.ts';
import { SVELTEKIT_GLOBAL_SPECIFIER } from '../constants.ts';
import type { Disknode } from './disknode.ts';

export type OnFilerChange = (change: WatcherChange, disknode: Disknode) => void;

export interface FilerOptions {
	watch_dir?: typeof watch_dir;
	watch_dir_options?: Partial<OmitStrict<WatchDirOptions, 'on_change'>>;
	log?: Logger;
}

export class Filer {
	readonly root_dir: PathId;

	// TODO rename everything to `disknode`
	readonly files: Map<PathId, Disknode> = new Map();

	#watch_dir: typeof watch_dir;
	#watch_dir_options: Partial<WatchDirOptions>;

	#log?: Logger;

	#listeners: Set<OnFilerChange> = new Set();
	#watching: WatchNodeFs | undefined;
	#initing: Promise<void> | undefined;
	#closing: Promise<void> | undefined;
	// set by `close` or a failed `init` until the next `init`,
	// the one check that stops processing: every queued change is skipped while it's set
	#closed = false;

	#change_queue: Array<WatcherChange> = [];
	// the external nodes never read, reached only by bare specifiers so far
	#unread: Set<PathId> = new Set();
	#processing_promise: Promise<void> | null = null;

	constructor(options: FilerOptions = EMPTY_OBJECT) {
		this.#watch_dir = options.watch_dir ?? watch_dir;
		this.#watch_dir_options = options.watch_dir_options ?? EMPTY_OBJECT;
		this.root_dir = resolve(options.watch_dir_options?.dir ?? paths.source);
		// TODO for package.json maybe another array of files/dirs to watch to invalidate everything?
		// or instead of that, think of taking an array of config objects that can specify invalidation rules,
		// so package.json would be configured differently than ./src, and we could add a default with
		// package.json/gro.config.ts/tsconfig.json/vite.config.ts to invalidate everything
		this.#log = options.log;
	}
	get inited(): boolean {
		return this.#watching !== undefined;
	}

	get_by_id = (id: PathId): Disknode | undefined => {
		return this.files.get(id);
	};

	/**
	 * Gets the node for `id`, creating it if needed.
	 * @param read - whether an external node is read and parsed, `false` for a file
	 *   reached by a bare specifier, which stays a contentless node until a path or `#` import
	 *   of the same file asks for it to be read
	 */
	get_or_create = (id: PathId, read = true): Disknode => {
		const existing = this.get_by_id(id);
		if (existing) {
			// a file first reached by a bare specifier is read once something imports it by path
			if (read && this.#unread.delete(id)) this.#queue_external_add(id);
			return existing;
		}
		const file: Disknode = {
			id,
			contents: null,
			external: this.#is_external(id), // TODO maybe filter externals by default? the user needs to configure the filer then
			ctime: null,
			mtime: null,
			content_hash: null,
			dependents: new Map(),
			dependencies: new Map()
		};
		this.files.set(id, file);
		if (file.external) {
			if (read) this.#queue_external_add(id);
			else this.#unread.add(id);
		}
		return file;
	};

	// Defer external file change notification to avoid reentrancy during queue processing
	#queue_external_add(id: PathId): void {
		queueMicrotask(() => {
			this.#on_change({ type: 'add', path: id, is_directory: false });
		});
	}

	filter(predicate: (disknode: Disknode) => boolean): Array<Disknode> | null {
		let found: Array<Disknode> | null = null;
		for (const disknode of this.files.values()) {
			if (predicate(disknode)) {
				(found ??= []).push(disknode);
			}
		}
		return found;
	}

	/**
	 * Initialize the filer to populate files without watching.
	 * Safe to call multiple times - subsequent calls are no-ops.
	 * Used by gen files to access the file graph.
	 */
	async init(): Promise<void> {
		// a close in flight cleans up when it finishes, so let it before initing again
		if (this.#closing) await this.#closing.catch(() => {});

		// if already initing, return the existing promise
		if (this.#initing) return this.#initing;

		// if already initialized, just ensure ready
		if (this.#watching) {
			return this.#watching.init();
		}

		// start new initialization
		this.#closed = false;
		this.#initing = this.#init();
		try {
			await this.#initing;
		} catch (error) {
			// stop processing before the shared cleanup, like `close`
			this.#closed = true;
			await this.#processing_promise;
			this.#cleanup();
			throw error;
		} finally {
			this.#initing = undefined;
		}
	}

	async #init(): Promise<void> {
		const watcher = this.#watch_dir({
			...this.#watch_dir_options,
			dir: this.root_dir,
			on_change: this.#on_change
		});

		try {
			await watcher.init();

			// Wait for any queued changes from init to be processed
			await this.#drain_queue();

			// check if close() was called during init
			if (this.#closing) {
				await watcher.close();
				return;
			}

			// only set after successful init and not closing
			this.#watching = watcher;
		} catch (error) {
			// clean up watcher on error, but don't let close error mask init error
			try {
				await watcher.close();
			} catch {
				// ignore close errors - init error is more important
			}
			throw error;
		}
	}

	async watch(listener: OnFilerChange): Promise<() => void> {
		await this.#add_listener(listener);
		return () => {
			this.#remove_listener(listener);
		};
	}

	/**
	 * Internal cleanup of all state - can be called safely from anywhere
	 */
	#cleanup(): void {
		this.#listeners.clear();
		this.files.clear();
		this.#unread.clear();
		this.#watching = undefined;
		this.#change_queue = [];
		// #initing is handled in finally block of init(),
		// and #processing_promise by the run that set it, which closing awaits
	}

	close(): Promise<void> {
		// if already closing, return existing promise
		if (this.#closing) return this.#closing;

		// if already closed and not initing, nothing to do
		if (!this.#watching && !this.#initing) return Promise.resolve();

		// start new close operation, stopping any change from processing
		this.#closed = true;
		const closing = this.#close();
		this.#closing = closing;
		// Clean up after completion, but don't change the returned promise
		// Use void to ensure we don't accidentally return the .then() promise
		void closing.then(
			() => {
				this.#closing = undefined;
			},
			() => {
				this.#closing = undefined;
			}
		);
		return this.#closing;
	}

	async #close(): Promise<void> {
		// wait for any pending initialization to complete
		if (this.#initing) {
			try {
				await this.#initing;
			} catch {
				// ignore errors during close
			}
		}

		// close watcher if it exists
		if (this.#watching) {
			await this.#watching.close();
		}

		// wait for the in-flight run, which skips the rest of the queue,
		// so the graph doesn't change after closing
		await this.#processing_promise;

		// clean up all state
		this.#cleanup();
	}

	async #update(id: PathId): Promise<Disknode | null> {
		const file = this.get_or_create(id);

		let stats: Awaited<ReturnType<typeof stat>> | null = null;
		let new_bytes: Buffer | null = null; // TODO need to lazily load contents, probably turn `Disknode` into a class

		try {
			stats = await stat(id);
			// only a regular file is read: a directory (`import '../../'`) or a special file
			// like a FIFO, which a read would block on, is a contentless node like a missing file
			if (stats.isFile()) new_bytes = await readFile(id);
		} catch (error) {
			const code = (error as NodeJS.ErrnoException).code;
			// treat the file as deleted or inaccessible for common error codes;
			// `ENOTDIR` is a dependency specifier naming a path through a file (`import '../x.ts/y'`),
			// and `EISDIR` a file replaced by a directory between the stat and the read
			if (
				code === 'ENOENT' ||
				code === 'EACCES' ||
				code === 'EPERM' ||
				code === 'EISDIR' ||
				code === 'ENOTDIR'
			) {
				// the file doesn't exist, is inaccessible, or is not a file,
				// so a read failing after the stat leaves no times either
				stats = null;
			} else {
				throw error;
			}
		}

		// hash the raw bytes, since decoding collapses invalid UTF-8 in binary files to U+FFFD
		const new_hash = new_bytes !== null ? hash_blake3(new_bytes) : null;

		file.ctime = stats?.ctimeMs ?? null;
		file.mtime = stats?.mtimeMs ?? null;

		// Use hash comparison for change detection (content-based, not mtime-based)
		if (file.content_hash === new_hash) {
			return null;
		}

		file.contents = new_bytes !== null ? new_bytes.toString('utf8') : null;
		file.content_hash = new_hash;

		const dependencies_before = new Set(file.dependencies.keys());
		const dependencies_removed = new Set(dependencies_before);

		let imported: Array<string> = [];
		if (file.contents) {
			try {
				imported = parse_imports(file.id, file.contents);
			} catch (error) {
				this.#log?.error('[filer] Failed to parse imports', file.id, error);
			}
		}
		for (const specifier of imported) {
			if (SVELTEKIT_GLOBAL_SPECIFIER.test(specifier)) continue;
			let path_id;
			// a file reached by a bare specifier, a package's, is never read or parsed,
			// since nothing reads a package's contents and its edge is all a dependent needs,
			// even when it's linked from outside `node_modules`;
			// a subpath import like `#lib/x.ts` names a project file, so it's read,
			// and so is a file a path or subpath import also reaches, whichever came first
			let read = true;
			if (is_path_specifier(specifier)) {
				// the exact named path, existing or not, so a file created later links to its importers
				path_id = to_specifier_path_id(specifier, file.id);
			} else {
				const file_url = pathToFileURL(file.id);
				let resolved_url;
				try {
					resolved_url = import.meta.resolve(specifier, file_url.href);
				} catch (error) {
					this.#log?.error('[filer] failed to resolve path', specifier, file_url.href, error);
					continue;
				}
				if (!resolved_url.startsWith('file:')) continue; // skip node:, npm:, https:, jsr:, etc.
				path_id = fileURLToPath(resolved_url);
				read = specifier[0] === '#';
			}
			dependencies_removed.delete(path_id);
			// an existing edge may now be a path or `#` import asking for the read
			const d = this.get_or_create(path_id, read);
			if (!dependencies_before.has(path_id)) {
				file.dependencies.set(d.id, d);
				d.dependents.set(file.id, file);
			}
		}

		// update any removed dependencies
		for (const dependency_removed of dependencies_removed) {
			file.dependencies.delete(dependency_removed);
			this.get_by_id(dependency_removed)?.dependents.delete(file.id);
		}

		return file;
	}

	#remove(id: PathId): Disknode | null {
		const file = this.get_by_id(id);
		if (!file) return null; // this is safe because the object would exist if any other file referenced it as a dependency or dependent

		// a node with no contents, like one already deleted by a read that found no file,
		// a missing import, or a FIFO, changes nothing, so it's no deletion to notify
		const had_contents = file.content_hash !== null;

		file.contents = null; // clear contents in case it gets re-added later, we want the change to be detected
		file.content_hash = null; // clear hash so re-add detects the change

		// a deleted file imports nothing, so it's no longer a dependent of what it imported
		for (const dependency_id of file.dependencies.keys()) {
			this.get_by_id(dependency_id)?.dependents.delete(id);
		}
		file.dependencies.clear();

		// keep the file in memory if other files still depend on it
		if (file.dependents.size === 0) {
			this.files.delete(id);
		}

		return had_contents ? file : null;
	}

	#sync_listener_with_files(listener: OnFilerChange): void {
		for (const disknode of this.files.values()) {
			try {
				listener({ type: 'add', path: disknode.id, is_directory: false }, disknode);
			} catch (error) {
				this.#log?.error('[filer] Listener error during sync:', error);
			}
		}
	}

	#notify_change(change: WatcherChange, disknode: Disknode): void {
		for (const listener of this.#listeners) {
			try {
				listener(change, disknode);
			} catch (error) {
				this.#log?.error('[filer] Listener error during change notification:', error);
			}
		}
	}

	async #add_listener(listener: OnFilerChange): Promise<void> {
		this.#listeners.add(listener);

		// ensure initialized
		await this.init();
		// a close in flight, which `init` waits for, clears the listeners
		this.#listeners.add(listener);

		// notify of existing files
		this.#sync_listener_with_files(listener);
	}

	#remove_listener(listener: OnFilerChange): void {
		this.#listeners.delete(listener);
		// keep watching active even with no listeners, only close() tears down
	}

	async #drain_queue(): Promise<void> {
		// Wait for queue to be empty and no active processing
		while (this.#change_queue.length > 0 || this.#processing_promise) {
			await this.#process_queue();
		}
	}

	async #process_queue(): Promise<void> {
		// If already processing, return the existing promise
		if (this.#processing_promise) return this.#processing_promise;

		// Create and track the processing promise, which never rejects:
		// a failure, like an unexpected read error, is logged once, here,
		// so it never fails `init` or reaches a caller sharing the promise
		const processing = this.#do_process_queue().catch((error: unknown) => {
			(this.#log ?? console).error('[filer] failed to process changes', error);
		});
		this.#processing_promise = processing;
		await processing;
		this.#processing_promise = null;

		// a change queued after the loop drained but before the reset above,
		// like the rest of a burst of synchronous deletes, or the changes behind a failure;
		// each change leaves the queue before it's processed, so this can't loop on one
		if (this.#change_queue.length > 0) await this.#process_queue();
	}

	async #do_process_queue(): Promise<void> {
		while (this.#change_queue.length > 0) {
			const change = this.#change_queue.shift()!;

			if (this.#closed) continue; // ignore changes during and after close
			if (change.is_directory) continue; // TODO manage directories?

			let disknode: Disknode | null;
			switch (change.type) {
				case 'add':
				case 'update': {
					disknode = await this.#update(change.path);
					break;
				}
				case 'delete': {
					disknode = this.#remove(change.path);
					break;
				}
				default:
					throw new UnreachableError(change.type);
			}

			if (disknode && this.#listeners.size > 0) {
				this.#notify_change(change, disknode);
			}
		}
	}

	#on_change: WatcherChangeCallback = (change) => {
		// Enqueue the change (sync callback from chokidar)
		this.#change_queue.push(change);

		// Start processing if not already running; a running `#process_queue`
		// takes the change, as it rechecks the queue after its loop
		void this.#process_queue();
	};

	#is_external(id: PathId): boolean {
		const { filter } = this.#watch_dir_options;
		return !id.startsWith(this.root_dir + '/') || (!!filter && !filter(id, false));
	}
}

// TODO maybe `Disknode` class?
export const filter_dependents = (
	disknode: Disknode,
	get_by_id: (id: PathId) => Disknode | undefined,
	filter?: FileFilter,
	results: Set<PathId> = new Set(),
	searched: Set<PathId> = new Set(),
	log?: Logger
): Set<PathId> => {
	// Use iterative approach to avoid stack overflow on deep dependency trees
	const stack = [disknode];

	while (stack.length > 0) {
		const current = stack.pop()!;
		for (const dependent_id of current.dependents.keys()) {
			if (searched.has(dependent_id)) continue;
			searched.add(dependent_id);
			if (!filter || filter(dependent_id)) {
				results.add(dependent_id);
			}
			const dependent_disknode = get_by_id(dependent_id);
			if (!dependent_disknode) {
				log?.warn(
					`[filer.filter_dependents] dependent source file ${dependent_id} not found for ${
						current.id
					}`
				);
				continue;
			}
			stack.push(dependent_disknode);
		}
	}
	return results;
};
