import type { PathId } from '@fuzdev/fuz_util/path.ts';

// TODO extract more here from Filer

export interface Disknode {
	id: PathId;
	// TODO figure out the best API that makes this lazy
	/**
	 * The file decoded as UTF-8. Invalid sequences, as in binary files, decode to U+FFFD.
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
	 * BLAKE3 hash of the file's raw bytes, so binary files compare exactly.
	 * `null` iff `contents` is `null`.
	 * Used for content-based change detection and caching.
	 */
	content_hash: string | null;
	dependents: Map<PathId, Disknode>;
	dependencies: Map<PathId, Disknode>;
}
