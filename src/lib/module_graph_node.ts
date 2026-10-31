import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { FileChangeCallback, ModuleGraph, Watcher, WatchOptions } from './module_graph.ts';
import type { Filer, filter_dependents } from './internal/filer.ts';
import type { WatchNodeFs } from './internal/watch_dir.ts';

interface LoadedFiler {
	filer: Filer;
	filter_dependents: typeof filter_dependents;
}

export interface NodeModuleGraphOptions {
	watch: WatchOptions;
	log?: Logger;
}

/**
 * Creates the Node `ModuleGraph`, over the `Filer`.
 * The `Filer` (and so chokidar and the import parser) is imported on first use,
 * so a task that never queries the graph never loads them.
 */
export const create_node_module_graph = (options: NodeModuleGraphOptions): ModuleGraph => {
	const { watch, log } = options;
	let loading: Promise<LoadedFiler> | undefined;
	let closed = false;
	let closing: Promise<void> | undefined;

	// re-checks `closed` after the await, so a query racing `close` rejects
	// instead of reading a cleaned-up Filer
	const load = async (): Promise<LoadedFiler> => {
		if (closed) throw Error('ModuleGraph is closed');
		loading ??= (async () => {
			const { Filer, filter_dependents } = await import('./internal/filer.ts');
			const filer = new Filer({ watch_dir_options: { dir: watch.dir, filter: watch.filter }, log });
			await filer.init();
			return { filer, filter_dependents };
		})();
		const loaded = await loading;
		if (closed) throw Error('ModuleGraph is closed');
		return loaded;
	};

	return {
		modules: async (filter) => {
			const { filer } = await load();
			const ids: Array<PathId> = [];
			for (const disknode of filer.files.values()) {
				if (disknode.external || disknode.contents === null) continue;
				if (filter && !filter(disknode.id)) continue;
				ids.push(disknode.id);
			}
			return ids;
		},
		dependents: async (id, filter) => {
			const { filer, filter_dependents } = await load();
			const disknode = filer.get_by_id(id);
			if (!disknode) return new Set();
			return filter_dependents(disknode, filer.get_by_id, filter, undefined, undefined, log);
		},
		watch: async (on_change: FileChangeCallback) => {
			const { filer } = await load();
			const unwatch = await filer.watch((change, disknode) => {
				if (disknode.external) return;
				// the Filer holds imported paths that don't exist, so a file created later links
				if (change.type === 'add' && disknode.contents === null) return;
				on_change({ type: change.type, id: disknode.id });
			});
			if (closed) {
				unwatch();
				throw Error('ModuleGraph is closed');
			}
			return () => {
				unwatch();
				return Promise.resolve();
			};
		},
		close: () =>
			(closing ??= (async () => {
				closed = true;
				if (!loading) return;
				const l = loading;
				loading = undefined;
				let loaded: LoadedFiler;
				try {
					loaded = await l;
				} catch {
					return; // a failed load already cleaned up
				}
				await loaded.filer.close();
			})())
	};
};

export interface NodeWatcherOptions {
	watch: WatchOptions;
}

/**
 * Creates the Node `Watcher`, a chokidar watch per `watch` call.
 * chokidar is imported on first use.
 */
export const create_node_watcher = (options: NodeWatcherOptions): Watcher => {
	const { watch } = options;
	const watching: Set<WatchNodeFs> = new Set();
	// watches not yet started, which `close` waits for
	const starting: Set<Promise<boolean>> = new Set();
	let closed = false;
	let closing: Promise<void> | undefined;

	return {
		watch: async (on_change) => {
			if (closed) throw Error('Watcher is closed');
			const { watch_dir } = await import('./internal/watch_dir.ts');
			if (closed) throw Error('Watcher is closed');
			const watcher = watch_dir({
				dir: watch.dir,
				filter: watch.filter,
				chokidar: { ignoreInitial: true },
				on_change: (change) => {
					if (change.is_directory) return;
					on_change({ type: change.type, id: change.path });
				}
			});
			// `watch_dir`'s `init` never resolves when closed before ready,
			// so a watch that `close` ran during closes itself once ready
			const starting_watch = (async () => {
				await watcher.init();
				if (closed) {
					await watcher.close();
					return false;
				}
				watching.add(watcher);
				return true;
			})();
			starting.add(starting_watch);
			let started: boolean;
			try {
				started = await starting_watch;
			} finally {
				starting.delete(starting_watch);
			}
			if (!started) throw Error('Watcher is closed');
			return async () => {
				if (!watching.delete(watcher)) return;
				await watcher.close();
			};
		},
		close: () =>
			(closing ??= (async () => {
				closed = true;
				const closing_watches = Array.from(watching, (w) => w.close());
				watching.clear();
				// settle every teardown before rejecting, so an awaited `close` means fully closed
				const results = await Promise.allSettled([
					...closing_watches,
					...Array.from(starting, (s) => s.catch(() => false))
				]);
				const failed = results.find((r) => r.status === 'rejected');
				if (failed) throw failed.reason;
			})())
	};
};
