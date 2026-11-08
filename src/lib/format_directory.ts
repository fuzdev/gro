import { map_concurrent } from '@fuzdev/fuz_util/async.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathFilter } from '@fuzdev/fuz_util/path.ts';
import { isAbsolute, join, matchesGlob, relative, resolve } from 'node:path';
import { IgnoreStack } from '@fuzdev/tsv';

import { GRO_CONFIG_FILENAME, VITE_CONFIG_FILENAMES } from './constants.ts';
import { format_file } from './format_file.ts';
import { paths, to_search_filters_relative } from './paths.ts';
import type { FsDir } from './fs.ts';
import { is_fs_error } from './fs_error.ts';
import { fs_search } from './fs_search.ts';
import { fs_open_dir_path, fs_open_path } from './internal/fs_path.ts';

/**
 * File extensions the format sweep collects. `json` is intentionally omitted
 * for now — `format_file` can still format json when called directly (e.g. gen
 * output), but `gro format` leaves json files untouched, since whole-value
 * reserialization diverges from other tools (e.g. array wrapping).
 */
const FORMATTABLE_MATCHER = /\.(ts|mts|cts|js|mjs|cjs|svelte|css)$/;

/**
 * Root-level files formatted alongside the root's `src/`.
 * Every Vite config filename is listed rather than just the `.js`/`.ts` one,
 * since which extension a project uses is Vite's call; the ones a project
 * doesn't have are skipped when read, the same as any other default root file it lacks.
 * `package.json` is intentionally omitted — `gro sync` owns its serialization
 * via `package_json_serialize` (2-space, matching the npm convention).
 * `tsconfig.json` is omitted because the sweep no longer formats json.
 */
const ROOT_FILES_DEFAULT = [GRO_CONFIG_FILENAME, ...VITE_CONFIG_FILENAMES];

/**
 * The root ignore file for the format sweep (gitignore-style patterns).
 */
const FORMATIGNORE_FILENAME = '.formatignore';

/**
 * Read where the root has no `.formatignore`, with tsv's shadowing rule, so a repo carrying the
 * legacy file (or keeping it for editor Prettier) has its patterns respected.
 */
const PRETTIERIGNORE_FILENAME = '.prettierignore';

const FORMAT_CONCURRENCY = 16;

/**
 * A file to format, its id and the handle it's read and written through.
 */
interface FormatFile {
	id: string;
	dir: FsDir;
	// beneath `dir`
	path: string;
}

export interface FormatDirectoryResult {
	ok: boolean;
	/** The files that were formatted (or, in `check` mode, that need formatting). */
	formatted: Array<string>;
	/** Files the formatter rejected (e.g. a syntax error), left untouched. */
	errored: Array<{ id: string; error: unknown }>;
}

export interface FormatDirectoryOptions {
	log: Logger;
	/**
	 * The directory swept when `patterns` is empty.
	 */
	dir: string;
	/**
	 * When `true`, reports unformatted files instead of writing them.
	 * @default false
	 */
	check?: boolean;
	/**
	 * Directory filters (e.g. `config.search_filters`) to skip, tested relative to the project root,
	 * or outside it to the searched directory or a glob's leading literal directory; also prunes
	 * `patterns` results, so a glob within the root can't reach `node_modules`, while a glob whose
	 * literal base is outside the root (`../other/node_modules/x/*.js`) is taken as given.
	 */
	filter?: PathFilter | Array<PathFilter>;
	/**
	 * Files or globs to format instead of sweeping `dir`, relative to the cwd or absolute.
	 * A pattern naming an existing file is that file, followed through its symlinks,
	 * even when its name holds glob syntax (`src/routes/[slug]/+page.ts`).
	 * Any other is a glob, matched as `path.matchesGlob` does, searched from its leading
	 * literal directory, whose symlinks are followed; beneath it, a symlink is skipped.
	 */
	patterns?: Array<string>;
	/**
	 * The project root `filter` tests paths relative to, and where the ignore files are read.
	 * @default `paths.root`
	 */
	root?: string;
}

/**
 * Formats files on the filesystem in-process via `format_file`.
 * When `patterns` is provided, formats those specific files/globs.
 * Otherwise formats `dir` (recursively, respecting `filter`) plus the root
 * files when `dir` is the root's `src/`. A sweep acts on regular files only,
 * skipping a symlink, as tsv's own walk does, root files included.
 * In both modes, files matched by the project root's `.formatignore`,
 * or with none, its `.prettierignore`, are skipped.
 *
 * Files the formatter rejects (e.g. a syntax error), or that can't be read,
 * are reported and fail the run in both modes rather than being silently skipped.
 * A default root file the project lacks, or that's a symlink, is skipped.
 *
 * This is separated from `./format_file` so modules that only need directory
 * traversal don't pull in the formatter (which loads the `tsv` native addon).
 */
export const format_directory = async ({
	log,
	dir,
	check = false,
	filter,
	patterns,
	root = paths.root
}: FormatDirectoryOptions): Promise<FormatDirectoryResult> => {
	const [collected, is_ignored] = await Promise.all([
		patterns?.length
			? collect_glob_files(patterns, filter, root)
			: collect_source_files(dir, filter, root),
		read_format_ignore(root)
	]);
	const files = collected.filter((file) => !is_ignored(file.id));

	const formatted: Array<string> = [];
	const errored: Array<{ id: string; error: unknown }> = [];
	await map_concurrent(files, FORMAT_CONCURRENCY, async ({ id, dir, path }) => {
		let content: string | null;
		try {
			content = await dir.read_text_if_exists(path);
		} catch (error) {
			errored.push({ id, error });
			return;
		}
		if (content === null) return; // a default root file that doesn't exist in this project
		let next: string;
		try {
			next = format_file(content, { filepath: id });
		} catch (error) {
			errored.push({ id, error });
			return;
		}
		if (next === content) return;
		formatted.push(id);
		if (!check) await dir.write(path, next);
	});

	// `map_concurrent` completes in nondeterministic order; sort for stable output.
	formatted.sort();
	errored.sort((a, b) => a.id.localeCompare(b.id));

	if (check && formatted.length) {
		log.error(`${formatted.length} file(s) need formatting:`);
		for (const id of formatted) log.error(`  ${id}`);
	}
	if (errored.length) {
		log.error(`${errored.length} file(s) could not be formatted:`);
		for (const { id, error } of errored) {
			log.error(`  ${id}: ${error instanceof Error ? error.message : String(error)}`);
		}
	}

	return { ok: errored.length === 0 && (!check || formatted.length === 0), formatted, errored };
};

/**
 * Whether the format sweep skips a file, given its absolute id.
 */
export type FormatIgnore = (id: string) => boolean;

/**
 * Reads the root's ignore file into a matcher of gitignore-style patterns,
 * with tsv's shadowing rule: `.formatignore`, or where the root has none, `.prettierignore`.
 * Only the root's ignore files are read, in a git repo or out of one. The patterns are anchored at the root, as tsv's `IgnoreStack` matches them:
 * a pattern with no `/` matches at any depth, a pattern with `/` is anchored to the root,
 * a directory pattern skips everything beneath it, and `!` re-includes. A file outside
 * the root is never skipped, and no ignore file skips nothing.
 * @throws `FsError` - when reading an ignore file fails other than by its absence
 */
export const read_format_ignore = async (root: string): Promise<FormatIgnore> => {
	const read = async (name: string): Promise<string | null> => {
		const opened = await fs_open_path(join(root, name));
		return opened.dir.read_text_if_exists(opened.path);
	};
	const stack = new IgnoreStack();
	const formatignore = await read(FORMATIGNORE_FILENAME);
	if (formatignore !== null) {
		stack.push_formatignore('', formatignore);
	} else {
		const prettierignore = await read(PRETTIERIGNORE_FILENAME);
		if (prettierignore === null) return () => false;
		stack.push_prettierignore('', prettierignore);
	}
	const root_dir = resolve(root);
	return (id) => {
		const path = relative(root_dir, id);
		if (!path || path === '..' || path.startsWith('../') || isAbsolute(path)) return false;
		return stack.is_ignored(path, false);
	};
};

const GLOB_MAGIC_MATCHER = /[*?[\]{}()!]/;

/**
 * Splits a glob pattern into the directory it searches, its leading segments before the
 * first with glob syntax, and the segments after it.
 */
const to_glob_parts = (pattern: string): { base_dir: string; rest: Array<string> } => {
	const segments = pattern.split('/');
	const files = segments.pop()!; // the last segment names files, magic or not
	const magic_index = segments.findIndex((segment) => GLOB_MAGIC_MATCHER.test(segment));
	const base_segments = magic_index === -1 ? segments : segments.slice(0, magic_index);
	const base = base_segments.join('/');
	const rest = (magic_index === -1 ? [] : segments.slice(magic_index)).concat(files);
	return {
		base_dir: resolve(base || (pattern.startsWith('/') ? '/' : '.')),
		rest: rest.filter((segment) => segment !== '' && segment !== '.')
	};
};

/**
 * Finds the files the patterns name. A pattern naming an existing file is that file,
 * its symlinks followed, whatever glob syntax its name holds, like `[slug]`, and it skips
 * the `filter`; any other pattern with glob syntax is a glob, searched from its literal
 * base directory, descending no deeper than it can match unless it has a `**`, where only
 * regular files match, never a symlink.
 */
const collect_glob_files = async (
	patterns: Array<string>,
	filter: PathFilter | Array<PathFilter> | undefined,
	root: string
): Promise<Array<FormatFile>> => {
	const files: Map<string, FormatFile> = new Map();
	for (const pattern of patterns) {
		const literal = await open_file(pattern);
		if (literal !== null) {
			if (FORMATTABLE_MATCHER.test(literal.id) && !files.has(literal.id)) {
				files.set(literal.id, literal);
			}
			continue;
		}
		if (!GLOB_MAGIC_MATCHER.test(pattern)) continue; // names no file
		const { base_dir, rest } = to_glob_parts(pattern);
		const opened = await fs_open_dir_path(base_dir);
		if (opened === null) continue;
		const matcher = resolve(pattern);
		const unbounded = rest.some((segment) => segment.includes('**'));
		const base_prefix = base_dir === '/' ? '/' : base_dir + '/';
		// a directory at the depth of the pattern's last segment can hold no match
		const depth_filter: PathFilter = (id, is_directory) =>
			unbounded || !is_directory || id.slice(base_prefix.length).split('/').length < rest.length;
		const found = await fs_search(opened.dir, {
			path: opened.path,
			id_dir: base_dir,
			filter: [depth_filter, ...to_search_filters_relative(filter, root, base_dir)],
			file_filter: (id) => FORMATTABLE_MATCHER.test(id) && matchesGlob(id, matcher)
		});
		for (const f of found) {
			if (f.kind === 'file' && !files.has(f.id)) {
				files.set(f.id, { id: f.id, dir: opened.dir, path: join_beneath(opened.path, f.path) });
			}
		}
	}
	return Array.from(files.values());
};

/**
 * Opens the file a path names, its symlinks followed,
 * or `null` when nothing or something other than a file is there.
 */
const open_file = async (path: string): Promise<FormatFile | null> => {
	const id = resolve(path);
	try {
		const opened = await fs_open_path(id);
		const info = await opened.dir.stat(opened.path);
		return info?.kind === 'file' ? { id, ...opened } : null;
	} catch (error) {
		// a path through a file names no file
		if (is_fs_error(error, 'not_a_directory')) return null;
		throw error;
	}
};

const collect_source_files = async (
	dir: string,
	filter: PathFilter | Array<PathFilter> | undefined,
	root: string
): Promise<Array<FormatFile>> => {
	const files: Array<FormatFile> = [];
	// opened once, the found files read and written beneath it
	const opened = await fs_open_dir_path(dir);
	if (opened !== null) {
		const found = await fs_search(opened.dir, {
			path: opened.path,
			id_dir: resolve(dir),
			filter: to_search_filters_relative(filter, root, dir),
			file_filter: (path) => FORMATTABLE_MATCHER.test(path)
		});
		for (const f of found) {
			if (f.kind !== 'file') continue; // a symlink, which the sweep never follows
			files.push({ id: f.id, dir: opened.dir, path: join_beneath(opened.path, f.path) });
		}
	}
	if (dir === join(root, 'src/')) {
		// entries of the root, never followed, so a symlinked root file is skipped like a sweep's
		const root_dir = await fs_open_dir_path(root);
		if (root_dir !== null) {
			const root_files = await Promise.all(
				ROOT_FILES_DEFAULT.map(async (name): Promise<FormatFile | null> => {
					const path = join_beneath(root_dir.path, name);
					const info = await root_dir.dir.stat(path, { follow: false });
					return info?.kind === 'file' ? { id: join(root, name), dir: root_dir.dir, path } : null;
				})
			);
			for (const f of root_files) if (f !== null) files.push(f);
		}
	}
	return files;
};

const join_beneath = (dir_path: string, path: string): string =>
	dir_path ? dir_path + '/' + path : path;
