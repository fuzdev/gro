import {map_concurrent} from '@fuzdev/fuz_util/async.ts';
import {fs_search} from '@fuzdev/fuz_util/fs.ts';
import type {Logger} from '@fuzdev/fuz_util/log.ts';
import type {PathFilter} from '@fuzdev/fuz_util/path.ts';
import {globSync} from 'node:fs';
import {readFile, writeFile} from 'node:fs/promises';
import {isAbsolute, join, resolve} from 'node:path';

import {
	GRO_CONFIG_FILENAME,
	SVELTE_CONFIG_FILENAME,
	TSCONFIG_FILENAME,
	VITE_CONFIG_FILENAME,
} from './constants.ts';
import {format_file} from './format_file.ts';
import {paths} from './paths.ts';

/** Matches the file extensions `format_file` knows how to format. */
const FORMATTABLE_MATCHER = /\.(ts|mts|cts|js|mjs|cjs|svelte|css|json)$/;

/**
 * Root-level files formatted alongside `paths.source`.
 * `package.json` is intentionally omitted — `gro sync` owns its serialization
 * via `package_json_serialize` (2-space, matching the npm convention).
 */
const ROOT_FILES_DEFAULT = [
	GRO_CONFIG_FILENAME,
	SVELTE_CONFIG_FILENAME,
	VITE_CONFIG_FILENAME,
	TSCONFIG_FILENAME,
];

const FORMAT_CONCURRENCY = 16;

export interface FormatDirectoryResult {
	ok: boolean;
	/** The files that were formatted (or, in `check` mode, that need formatting). */
	formatted: Array<string>;
	/** Files the formatter rejected (e.g. a syntax error), left untouched. */
	errored: Array<{id: string; error: unknown}>;
}

/**
 * Formats files on the filesystem in-process via `format_file`.
 * When `patterns` is provided, formats those specific files/globs.
 * Otherwise formats `dir` (recursively, respecting `filter`) plus the root
 * files when `dir` is `paths.source`.
 *
 * Files the formatter rejects (e.g. a syntax error) are reported and fail the
 * run in both modes rather than being silently skipped.
 *
 * This is separated from `./format_file` so modules that only need directory
 * traversal don't pull in the formatter (which loads the `tsv` WASM module).
 *
 * @param check - when `true`, reports unformatted files instead of writing them
 * @param filter - directory filters (e.g. `config.search_filters`) to skip;
 * also prunes `patterns` results so an explicit glob can't reach `node_modules`
 */
export const format_directory = async (
	log: Logger,
	dir: string,
	check = false,
	filter?: PathFilter | Array<PathFilter>,
	patterns?: Array<string>,
): Promise<FormatDirectoryResult> => {
	const file_ids = patterns?.length
		? globSync(patterns, to_glob_options(filter))
				.filter((p) => FORMATTABLE_MATCHER.test(p))
				.map((p) => (isAbsolute(p) ? p : resolve(p)))
		: await collect_source_files(dir, filter);

	const formatted: Array<string> = [];
	const errored: Array<{id: string; error: unknown}> = [];
	await map_concurrent(file_ids, FORMAT_CONCURRENCY, async (id) => {
		let content: string;
		try {
			content = await readFile(id, 'utf8');
		} catch (_err) {
			return; // a default root file that doesn't exist in this project
		}
		let next: string;
		try {
			next = format_file(content, {filepath: id});
		} catch (error) {
			errored.push({id, error});
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
		for (const {id, error} of errored) {
			log.error(`  ${id}: ${error instanceof Error ? error.message : String(error)}`);
		}
	}

	return {ok: errored.length === 0 && (!check || formatted.length === 0), formatted, errored};
};

/** Builds `globSync` options that prune the same paths `filter` would exclude. */
const to_glob_options = (
	filter?: PathFilter | Array<PathFilter>,
): {exclude: (path: string) => boolean} => {
	const filters = filter && (Array.isArray(filter) ? filter : [filter]);
	if (!filters?.length) return {exclude: () => false};
	return {exclude: (path) => filters.some((f) => !f(path, false))};
};

const collect_source_files = async (
	dir: string,
	filter?: PathFilter | Array<PathFilter>,
): Promise<Array<string>> => {
	const found = await fs_search(dir, {
		filter,
		file_filter: (path) => FORMATTABLE_MATCHER.test(path),
	});
	const file_ids = found.map((f) => f.id);
	if (dir === paths.source) {
		for (const name of ROOT_FILES_DEFAULT) {
			file_ids.push(join(paths.root, name));
		}
	}
	return file_ids;
};
