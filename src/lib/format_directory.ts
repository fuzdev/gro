import { map_concurrent } from '@fuzdev/fuz_util/async.ts';
import { ensure_end } from '@fuzdev/fuz_util/string.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathFilter } from '@fuzdev/fuz_util/path.ts';
import { globSync, readFileSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';

import { GRO_CONFIG_FILENAME, VITE_CONFIG_FILENAMES } from './constants.ts';
import { format_file } from './format_file.ts';
import { paths, to_search_filters_relative } from './paths.ts';
import { fs_search_path } from './internal/fs_path.ts';

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
 * Root-level ignore files for the format sweep (gitignore-style patterns).
 * `.prettierignore` is honored alongside `.formatignore` so a repo carrying the
 * legacy file (or keeping it for editor Prettier) has its patterns respected.
 */
const FORMAT_IGNORE_FILENAMES = ['.formatignore', '.prettierignore'];

const FORMAT_CONCURRENCY = 16;

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
	 * Globs to format instead of sweeping `dir`, relative to the cwd or absolute.
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
 * files when `dir` is the root's `src/`. In both modes, files matched by the
 * project root's `.formatignore` or `.prettierignore` are skipped.
 *
 * Files the formatter rejects (e.g. a syntax error) are reported and fail the
 * run in both modes rather than being silently skipped.
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
	const collected = patterns?.length
		? collect_glob_files(patterns, filter, root)
		: await collect_source_files(dir, filter, root);

	const ignored = read_format_ignore(root);
	const file_ids = ignored.size ? collected.filter((id) => !ignored.has(id)) : collected;

	const formatted: Array<string> = [];
	const errored: Array<{ id: string; error: unknown }> = [];
	await map_concurrent(file_ids, FORMAT_CONCURRENCY, async (id) => {
		let content: string;
		try {
			content = await readFile(id, 'utf8');
		} catch (_err) {
			return; // a default root file that doesn't exist in this project
		}
		let next: string;
		try {
			next = format_file(content, { filepath: id });
		} catch (error) {
			errored.push({ id, error });
			return;
		}
		if (next === content) return;
		formatted.push(id);
		if (!check) await writeFile(id, next);
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
 * Reads the root ignore files (`.formatignore` and `.prettierignore`) and
 * expands their merged gitignore-style globs to a set of absolute paths the
 * format sweep skips. A pattern with no `/` matches at any depth (`*.md` becomes
 * `** /*.md`); a pattern with `/` is anchored to the root. Blank lines and `#`
 * comments are ignored, and absent files yield an empty set (skip nothing).
 */
export const read_format_ignore = (root: string): Set<string> => {
	const patterns: Array<string> = [];
	for (const name of FORMAT_IGNORE_FILENAMES) {
		let content: string;
		try {
			content = readFileSync(join(root, name), 'utf8');
		} catch {
			continue;
		}
		for (const line of content.split('\n')) {
			const trimmed = line.trim();
			if (trimmed !== '' && !trimmed.startsWith('#')) {
				patterns.push(trimmed.includes('/') ? trimmed : `**/${trimmed}`);
			}
		}
	}
	if (patterns.length === 0) return new Set();
	return new Set(globSync(patterns, { cwd: root }).map((p) => resolve(root, p)));
};

/**
 * Builds `globSync` options that prune the same paths `filter` would exclude,
 * testing each path relative to the project root like the sweep,
 * or, outside the root, relative to `base_dir`,
 * and never pruning the root, a `base_dir` outside it, or the directories above them
 * that an absolute or `../` pattern walks through.
 * Node's glob calls `exclude` with each directory's path, relative to the cwd or absolute
 * like the pattern, and with each entry's bare name under a `**`,
 * so a name is tested as itself, never resolved against a cwd that may be elsewhere.
 */
const to_glob_options = (
	filter: PathFilter | Array<PathFilter> | undefined,
	root: string,
	base_dir: string
): { exclude: (path: string) => boolean } => {
	const filters = to_search_filters_relative(filter, root, base_dir);
	if (!filters.length) return { exclude: () => false };
	const root_dir = ensure_end(resolve(root), '/');
	const base = ensure_end(resolve(base_dir), '/');
	const base_outside_root = !base.startsWith(root_dir);
	return {
		exclude: (path) => {
			if (!path.includes('/')) return filters.some((f) => !f(path, false));
			const id = resolve(path);
			const dir = ensure_end(id, '/');
			if (root_dir.startsWith(dir) || (base_outside_root && base.startsWith(dir))) return false;
			return filters.some((f) => !f(id, false));
		}
	};
};

const GLOB_MAGIC_MATCHER = /[*?[\]{}()!]/;

/**
 * The directory a glob pattern searches, its leading segments before the first with glob syntax.
 */
const to_glob_base_dir = (pattern: string): string => {
	const segments = pattern.split('/');
	segments.pop(); // the last segment names files, magic or not
	const magic_index = segments.findIndex((segment) => GLOB_MAGIC_MATCHER.test(segment));
	const base = (magic_index === -1 ? segments : segments.slice(0, magic_index)).join('/');
	return resolve(base || (pattern.startsWith('/') ? '/' : '.'));
};

const collect_glob_files = (
	patterns: Array<string>,
	filter: PathFilter | Array<PathFilter> | undefined,
	root: string
): Array<string> => {
	const file_ids: Set<string> = new Set();
	for (const pattern of patterns) {
		for (const p of globSync(pattern, to_glob_options(filter, root, to_glob_base_dir(pattern)))) {
			if (FORMATTABLE_MATCHER.test(p)) file_ids.add(isAbsolute(p) ? p : resolve(p));
		}
	}
	return Array.from(file_ids);
};

const collect_source_files = async (
	dir: string,
	filter: PathFilter | Array<PathFilter> | undefined,
	root: string
): Promise<Array<string>> => {
	const found = await fs_search_path(dir, {
		filter: to_search_filters_relative(filter, root, dir),
		file_filter: (path) => FORMATTABLE_MATCHER.test(path)
	});
	const file_ids = found.map((f) => f.id);
	if (dir === join(root, 'src/')) {
		for (const name of ROOT_FILES_DEFAULT) {
			file_ids.push(join(root, name));
		}
	}
	return file_ids;
};
