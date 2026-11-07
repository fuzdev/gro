import { watch, type FSWatcher } from 'chokidar';
import { create_deferred, type Deferred } from '@fuzdev/fuz_util/async.ts';
import type { PathFilter } from '@fuzdev/fuz_util/path.ts';

import type { FileChangeType } from '../module_graph.ts';

const TMP_FILE_PATTERN = /\.tmp\./;

export interface WatchNodeFs {
	init: () => Promise<void>;
	close: () => Promise<void>;
}

export interface WatcherChange {
	type: FileChangeType;
	path: string;
	is_directory: boolean;
}
export type WatcherChangeCallback = (change: WatcherChange) => void;

export interface WatchDirOptions {
	dir: string;
	on_change: WatcherChangeCallback;
	filter?: PathFilter | null | undefined;
	/**
	 * Called with each error chokidar emits, like `EACCES` reading a directory,
	 * which is otherwise thrown, crashing the process.
	 */
	on_error?: (error: unknown) => void;
}

/**
 * Watch for changes on the filesystem using chokidar.
 */
export const watch_dir = ({ dir, on_change, filter, on_error }: WatchDirOptions): WatchNodeFs => {
	let watcher: FSWatcher | undefined;
	let initing: Deferred<void> | undefined;

	return {
		init: async () => {
			if (initing) return initing.promise;
			initing = create_deferred();
			watcher = watch(dir, { ignored: TMP_FILE_PATTERN });
			watcher.on('add', (path) => {
				if (filter && !filter(path, false)) return;
				on_change({ type: 'add', path, is_directory: false });
			});
			watcher.on('addDir', (path) => {
				if (filter && !filter(path, true)) return;
				on_change({ type: 'add', path, is_directory: true });
			});
			watcher.on('change', (path, s) => {
				// never stats, which could throw in a listener for a path already gone;
				// chokidar reports `change` for files, with stats when it has them
				const is_directory = s?.isDirectory() ?? false;
				if (filter && !filter(path, is_directory)) return;
				on_change({ type: 'update', path, is_directory });
			});
			watcher.on('unlink', (path) => {
				if (filter && !filter(path, false)) return;
				on_change({ type: 'delete', path, is_directory: false });
			});
			watcher.on('unlinkDir', (path) => {
				if (filter && !filter(path, true)) return;
				on_change({ type: 'delete', path, is_directory: true });
			});
			if (on_error) watcher.on('error', on_error);
			// wait until ready
			watcher.once('ready', () => initing?.resolve());
			await initing.promise;
		},
		close: async () => {
			initing = undefined;
			if (!watcher) return;
			await watcher.close();
		}
	};
};
