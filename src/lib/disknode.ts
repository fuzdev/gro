import type {PathId} from '@fuzdev/fuz_util/path.ts';

// TODO extract more here from Filer

export interface Disknode {
	id: PathId;
	// TODO figure out the best API that makes this lazy
	/**
	 * `null` contents means it doesn't exist.
	 * We create the file in memory to track its dependents regardless of its existence on disk.
	 */
	contents: string | null;
	/**
	 * Is the source file outside of the `root_dir` or excluded by `watch_dir_options.filter`?
	 */
	external: boolean;
	ctime: number | null;
	mtime: number | null;
	/**
	 * SHA-256 hash of `contents`. `null` iff `contents` is `null`.
	 * Used for content-based change detection and caching.
	 */
	content_hash: string | null;
	dependents: Map<PathId, Disknode>;
	dependencies: Map<PathId, Disknode>;
}
