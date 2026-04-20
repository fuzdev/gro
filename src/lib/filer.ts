import {EMPTY_OBJECT} from '@fuzdev/fuz_util/object.js';
import {readFile, stat} from 'node:fs/promises';
import {dirname, resolve} from 'node:path';
import type {OmitStrict} from '@fuzdev/fuz_util/types.js';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {UnreachableError} from '@fuzdev/fuz_util/error.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {PackageJson} from '@fuzdev/fuz_util/package_json.js';
import type {FileFilter, PathId} from '@fuzdev/fuz_util/path.js';
import {hash_blake3} from '@fuzdev/fuz_util/hash_blake3.js';

import {
	watch_dir,
	type WatchNodeFs,
	type WatcherChange,
	type WatchDirOptions,
	type WatcherChangeCallback,
} from './watch_dir.ts';
import {paths} from './paths.ts';
import {parse_imports} from './parse_imports.ts';
import {resolve_specifier} from './resolve_specifier.ts';
import {default_svelte_config} from './svelte_config.ts';
import {map_sveltekit_aliases} from './sveltekit_helpers.ts';
import {SVELTEKIT_GLOBAL_SPECIFIER} from './constants.ts';
import type {Disknode} from './disknode.ts';

const aliases = Object.entries(default_svelte_config.alias);

export type OnFilerChange = (change: WatcherChange, disknode: Disknode) => void;

export interface FilerOptions {
	watch_dir?: typeof watch_dir;
	watch_dir_options?: Partial<OmitStrict<WatchDirOptions, 'on_change'>>;
	package_json_cache?: Record<string, PackageJson>;
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

	#change_queue: Array<WatcherChange> = [];
	#processing_promise: Promise<void> | null = null;

	constructor(options: FilerOptions = EMPTY_OBJECT) {
		this.#watch_dir = options.watch_dir ?? watch_dir;
		this.#watch_dir_options = options.watch_dir_options ?? EMPTY_OBJECT;
		this.root_dir = resolve(options.watch_dir_options?.dir ?? paths.source);
		// TODO for package.json maybe another array of files/dirs to watch to invalidate everything?
		// or instead of that, think of taking an array of config objects that can specify invalidation rules,
		// so package.json would be configured differently than ./src, and we could add a default with
		// package.json/gro.config.ts/tsconfig.json/svelte.config.js/vite.config.ts to invalidate everything
		this.#log = options.log;
	}
	get inited(): boolean {
		return this.#watching !== undefined;
	}

	get_by_id = (id: PathId): Disknode | undefined => {
		return this.files.get(id);
	};

	get_or_create = (id: PathId): Disknode => {
		const existing = this.get_by_id(id);
		if (existing) return existing;
		const file: Disknode = {
			id,
			contents: null,
			external: this.#is_external(id), // TODO maybe filter externals by default? the user needs to configure the filer then
			ctime: null,
			mtime: null,
			content_hash: null,
			dependents: new Map(),
			dependencies: new Map(),
		};
		this.files.set(id, file);
		// Defer external file change notification to avoid reentrancy during queue processing
		if (file.external) {
			queueMicrotask(() => {
				this.#on_change({type: 'add', path: file.id, is_directory: false});
			});
		}
		return file;
	};

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
		// if already initing, return the existing promise
		if (this.#initing) return this.#initing;

		// if already initialized, just ensure ready
		if (this.#watching) {
			return this.#watching.init();
		}

		// start new initialization
		this.#initing = this.#init();
		try {
			await this.#initing;
		} catch (error) {
			// use shared cleanup logic
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
			on_change: this.#on_change,
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
		this.#watching = undefined;
		this.#change_queue = [];
		this.#processing_promise = null;
		// #initing is handled in finally block of init()
	}

	close(): Promise<void> {
		// if already closing, return existing promise
		if (this.#closing) return this.#closing;

		// if already closed and not initing, nothing to do
		if (!this.#watching && !this.#initing) return Promise.resolve();

		// start new close operation
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
			},
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

		// clean up all state
		this.#cleanup();
	}

	async #update(id: PathId): Promise<Disknode | null> {
		const file = this.get_or_create(id);

		let stats: Awaited<ReturnType<typeof stat>> | null = null;
		let new_contents: string | null = null; // TODO need to lazily load contents, probably turn `Disknode` into a class

		try {
			[stats, new_contents] = await Promise.all([stat(id), readFile(id, 'utf8')]);
		} catch (error) {
			const code = (error as NodeJS.ErrnoException).code;
			// Treat file as deleted/inaccessible for common error codes
			if (code === 'ENOENT' || code === 'EACCES' || code === 'EPERM') {
				// File doesn't exist or is inaccessible, treat as deleted
			} else {
				throw error;
			}
		}

		// Compute hash for new contents
		const new_hash = new_contents !== null ? hash_blake3(new_contents) : null;

		file.ctime = stats?.ctimeMs ?? null;
		file.mtime = stats?.mtimeMs ?? null;

		// Use hash comparison for change detection (content-based, not mtime-based)
		if (file.content_hash === new_hash) {
			return null;
		}

		file.contents = new_contents;
		file.content_hash = new_hash;

		const dir = dirname(file.id);

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
			const path = map_sveltekit_aliases(specifier, aliases);

			let path_id;
			// TODO replace `resolve_specifier` with `import.meta.resolve` for local specifiers too
			// once we move to explicit extensions (Deno-compatible) - the .js→.ts Vite convention
			// is the only reason resolve_specifier is still needed here
			if (path[0] === '.' || path[0] === '/') {
				const resolved = await resolve_specifier(path, dir);
				path_id = resolved.path_id;
			} else {
				const file_url = pathToFileURL(file.id);
				let resolved_url;
				try {
					resolved_url = import.meta.resolve(path, file_url.href);
				} catch (error) {
					this.#log?.error('[filer] failed to resolve path', path, file_url.href, error);
					continue;
				}
				if (!resolved_url.startsWith('file:')) continue; // skip node:, npm:, https:, jsr:, etc.
				path_id = fileURLToPath(resolved_url);
			}
			dependencies_removed.delete(path_id);
			if (!dependencies_before.has(path_id)) {
				const d = this.get_or_create(path_id);
				file.dependencies.set(d.id, d);
				d.dependents.set(file.id, file);
			}
		}

		// update any removed dependencies
		for (const dependency_removed of dependencies_removed) {
			file.dependencies.delete(dependency_removed);
			const dependency_removed_file = this.get_or_create(dependency_removed);
			dependency_removed_file.dependents.delete(file.id);
		}

		return file;
	}

	#remove(id: PathId): Disknode | null {
		const file = this.get_by_id(id);
		if (!file) return null; // this is safe because the object would exist if any other file referenced it as a dependency or dependent

		file.contents = null; // clear contents in case it gets re-added later, we want the change to be detected
		file.content_hash = null; // clear hash so re-add detects the change

		file.dependencies.clear();

		// keep the file in memory if other files still depend on it
		if (file.dependents.size === 0) {
			this.files.delete(id);
		}

		return file;
	}

	#sync_listener_with_files(listener: OnFilerChange): void {
		for (const disknode of this.files.values()) {
			try {
				listener({type: 'add', path: disknode.id, is_directory: false}, disknode);
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

		// Create and track the processing promise
		this.#processing_promise = this.#do_process_queue();

		try {
			await this.#processing_promise;
		} finally {
			this.#processing_promise = null;
		}
	}

	async #do_process_queue(): Promise<void> {
		while (this.#change_queue.length > 0) {
			const change = this.#change_queue.shift()!;

			if (this.#closing) continue; // ignore changes during close
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

		// Start processing if not already running
		void this.#process_queue();
	};

	#is_external(id: PathId): boolean {
		const {filter} = this.#watch_dir_options;
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
	log?: Logger,
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
					`[filer.filter_dependents] dependent source file ${dependent_id} not found for ${current.id}`,
				);
				continue;
			}
			stack.push(dependent_disknode);
		}
	}
	return results;
};
