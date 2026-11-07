import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { FileChangeCallback, ModuleGraph, WatchOptions } from './module_graph.ts';
import type { Filer, filter_dependents } from './internal/filer.ts';

interface LoadedFiler {
	filer: Filer;
	filter_dependents: typeof filter_dependents;
}

/**
 * Options for `create_node_module_graph`.
 */
export interface NodeModuleGraphOptions {
	/**
	 * What the graph watches, usually `GroConfig.watch`.
	 */
	watch: WatchOptions;
	/**
	 * The logger the graph reports through, like a file whose imports fail to parse,
	 * which also warns of a watch error, like `EACCES` reading a directory;
	 * absent, a watch error warns through `console`,
	 * and so does a failure processing a change, like an unexpected read error.
	 */
	log?: Logger;
}

/**
 * Creates the Node `ModuleGraph`, which watches `watch.dir` with chokidar
 * and reads each module's imports, following them outside `watch.dir`,
 * except that a file reached by a bare specifier, like a package's even when linked,
 * is an edge to a file never read, so its imports aren't edges.
 * Its implementation (with chokidar and the import parser) is imported on first use,
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
			const filer = new Filer({
				dir: watch.dir,
				filter: watch.filter,
				on_error: (error) => (log ?? console).warn('watch error, changes may be missed:', error),
				log
			});
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
			return filter_dependents(disknode, filer.get_by_id, filter, log);
		},
		watch: async (on_change: FileChangeCallback) => {
			const { filer } = await load();
			const unwatch = await filer.watch((change, disknode) => {
				if (disknode.external) return;
				if (change.type !== 'delete' && disknode.contents === null) {
					// the Filer holds imported paths that don't exist, so a file created later links
					if (change.type === 'add') return;
					// a change whose read found no file, like a save racing a delete, is a deletion
					on_change({ type: 'delete', id: disknode.id });
					return;
				}
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
