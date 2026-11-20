import { join } from 'node:path';

import type { FsPaths } from './fs_paths.ts';
import type { FsSearchResult } from './fs_search.ts';

/**
 * The file pattern `gro test` passes to Vitest by default,
 * which keeps the files whose path beneath the test directory contains it, ignoring case.
 */
export const TEST_FILE_PATTERN = '.test.';

// Vitest's default `include`, matched against a file's name
const VITEST_INCLUDE_MATCHER = /\.(?:test|spec)\.[cm]?[jt]sx?$/u;

// the directory names Vitest's default `exclude` skips, at any depth
const VITEST_EXCLUDED_NAMES: ReadonlySet<string> = new Set(['node_modules', '.git']);

/**
 * Finds the files `gro test` would run in `dir` with its default pattern,
 * by Vitest's own rules rather than Gro's `search_filters`, which Vitest never sees:
 * a test beneath a `build/` or `.`-prefixed directory runs, so it's found here too.
 *
 * Symlinks are followed as Vitest follows them, through `fs_paths`, so a link out of the project
 * reaches what it points at, and a dangling one is skipped. A path is matched as it's named
 * through the links, and the pattern can match a directory's name, so each real directory is
 * searched at most once with and once without a matching name above it. That finds every file
 * Vitest would and ends on a loop, which Vitest follows one level in.
 *
 * @param fs_paths - opens `dir` and each symlink beneath it
 * @param dir - the test directory, absolute
 * @returns the test files sorted by `path`, named through the links, with `kind` the listed one;
 * an empty array when nothing is at `dir`
 * @throws `FsError` - when `dir` isn't a directory, or a listing or a symlink's resolution fails
 */
export const find_test_files = async (
	fs_paths: FsPaths,
	dir: string
): Promise<Array<FsSearchResult>> => {
	const root = await fs_paths.open_dir_path(dir);
	if (root === null || (await root.dir.stat(root.path)) === null) return [];

	const found: Array<FsSearchResult> = [];
	// each real directory with whether a directory name above it matched the pattern
	const searched: Set<string> = new Set();

	const search = async (relative_dir: string, named_test: boolean): Promise<void> => {
		const opened = relative_dir ? await fs_paths.open_dir_path(join(dir, relative_dir)) : root;
		if (opened === null) return;
		const key = (named_test ? '1' : '0') + join(opened.dir.path, opened.path);
		if (searched.has(key)) return;
		searched.add(key);
		const subdirs: Array<[string, boolean]> = [];
		const links: Array<[string, string]> = [];
		const add = (path: string, name: string, kind: FsSearchResult['kind']): void => {
			if ((named_test || has_test_pattern(name)) && VITEST_INCLUDE_MATCHER.test(name)) {
				found.push({ path, id: join(dir, path), is_directory: false, kind });
			}
		};
		for await (const { name, kind } of opened.dir.list(opened.path)) {
			if (VITEST_EXCLUDED_NAMES.has(name)) continue;
			const path = relative_dir ? relative_dir + '/' + name : name;
			if (kind === 'directory') subdirs.push([path, named_test || has_test_pattern(name)]);
			else if (kind === 'symlink') links.push([path, name]);
			else add(path, name, kind);
		}
		// resolved after the listing, so its handle closes first
		for (const [path, name] of links) {
			const target = await fs_paths.open_path(join(dir, path));
			const info = await target.dir.stat(target.path);
			if (info === null) continue; // dangling, which Vitest skips
			if (info.kind === 'directory') subdirs.push([path, named_test || has_test_pattern(name)]);
			else add(path, name, 'symlink');
		}
		await Promise.all(subdirs.map(([path, named]) => search(path, named)));
	};
	await search('', false);

	return found.sort((a, b) => a.path.localeCompare(b.path));
};

// Vitest matches the pattern against the path ignoring case, and it holds no `/`,
// so it matches within a single name
const has_test_pattern = (name: string): boolean => name.toLowerCase().includes(TEST_FILE_PATTERN);
