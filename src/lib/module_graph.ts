import type { PathFilter, PathId } from '@fuzdev/fuz_util/path.ts';

/**
 * The project's module graph: which files exist under the watched directory
 * and which import which. Tasks and plugins get one on their context.
 * Queries load the graph on first use, and `close` tears it down for good.
 * A load that fails stays failed: later queries reject with the same error.
 */
export interface ModuleGraph {
	/**
	 * Gets the ids of the project modules that exist on disk.
	 * @param filter - narrows the results
	 */
	modules: (filter?: (id: PathId) => boolean) => Promise<Array<PathId>>;
	/**
	 * Gets every module that imports `id`, directly or transitively.
	 * The filter narrows the results, not the traversal.
	 * @param id - the module whose dependents to find
	 * @param filter - narrows the results
	 * @returns the dependents, including of an imported path that doesn't exist yet; empty when nothing imports `id`
	 */
	dependents: (id: PathId, filter?: (id: PathId) => boolean) => Promise<Set<PathId>>;
	/**
	 * Subscribes to changes of project modules, first calling `on_change`
	 * with an `add` for each module already in the graph.
	 * Changes are content-based, so saving a file unchanged emits nothing.
	 * @param on_change - called for each change
	 * @returns a function that unsubscribes
	 */
	watch: (on_change: FileChangeCallback) => Promise<() => Promise<void>>;
	/**
	 * Stops watching and drops the graph. A graph that never loaded closes without loading.
	 * Closing is permanent: later queries and watches reject, as do any pending when it's called.
	 * A `watch` pending when it's called may already have delivered its initial `add`s before rejecting.
	 * Every call returns the same teardown, so an awaited `close` means fully closed.
	 */
	close: () => Promise<void>;
}

/**
 * Watches the filesystem under the watched directory, with no module graph.
 */
export interface Watcher {
	/**
	 * Subscribes to file changes from now on. Files that already exist emit nothing.
	 * @param on_change - called for each file change, never for directories
	 * @returns a function that stops this watch
	 */
	watch: (on_change: FileChangeCallback) => Promise<() => Promise<void>>;
	/**
	 * Stops every watch this watcher started.
	 * Closing is permanent: later watches reject, as do any still starting.
	 * Every call returns the same teardown, so an awaited `close` means fully closed.
	 */
	close: () => Promise<void>;
}

/**
 * A file added, updated, or deleted, by absolute path.
 */
export interface FileChange {
	type: FileChangeType;
	id: PathId;
}

/**
 * The kind of a `FileChange`.
 */
export type FileChangeType = 'add' | 'update' | 'delete';

/**
 * Receives each `FileChange` of a watch.
 */
export type FileChangeCallback = (change: FileChange) => void;

/**
 * What a `ModuleGraph` and `Watcher` watch, from `GroConfig.watch`.
 */
export interface WatchOptions {
	/**
	 * The directory to watch, which holds the project modules.
	 * Files outside it enter the graph only when a project module imports them.
	 */
	dir: PathId;
	/**
	 * Excludes the paths it rejects from the change events and from the project modules.
	 * It's called with absolute paths, unlike `GroConfig.search_filters`,
	 * which get paths relative to the project root.
	 * It filters events, not the directory walk: a rejected directory is still watched,
	 * so each path below it is filtered on its own.
	 */
	filter: PathFilter | null;
}
