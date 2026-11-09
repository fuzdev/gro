import { join } from 'node:path';
import { createRequire } from 'node:module';
import { plural, strip_end } from '@fuzdev/fuz_util/string.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { PackageJson, PackageJsonExports } from '@fuzdev/fuz_util/package_json.ts';

import {
	PACKAGE_JSON_FILENAME,
	SVELTEKIT_DIST_DIRNAME,
	TS_MATCHER,
	JS_MATCHER,
	SVELTE_MATCHER,
	JSON_MATCHER,
	CSS_MATCHER,
	LIB_PATH
} from './constants.ts';
import { has_sveltekit_library } from './sveltekit_helpers.ts';
import { GITHUB_REPO_MATCHER } from './github.ts';
import { is_fs_error } from './fs_error.ts';
import type { FsPaths } from './fs_paths.ts';

export type PackageJsonMapper = (
	package_json: PackageJson
) => PackageJson | null | Promise<PackageJson | null>;

export const PACKAGE_JSON_EMPTY: PackageJson = { name: '', version: '' };

/**
 * Loads the `package.json` in `dir`, or `PACKAGE_JSON_EMPTY` when it can't be read or parsed as JSON,
 * or, when not `parse`, holds no object.
 * @param fs_paths - opens `dir`'s `package.json`
 * @param dir - absolute, or relative to the project; defaults to the project of `fs_paths`
 * @param cache - keyed by `dir` as given
 * @param parse - whether to validate it against the `PackageJson` schema
 * @param log - logs a failure to load, other than a missing file
 * @throws when `parse` and the loaded JSON fails the schema
 */
export const package_json_load = async (
	fs_paths: FsPaths,
	dir: string = fs_paths.project.path,
	cache?: Record<string, PackageJson>,
	parse = true, // TODO pass `false` here in more places, especially anything perf-sensitive like work on startup
	log?: Logger
): Promise<PackageJson> => {
	let package_json: PackageJson;
	if (cache && dir in cache) {
		return cache[dir]!;
	}
	try {
		package_json = JSON.parse(await package_json_load_contents(fs_paths, dir));
	} catch (error) {
		if (!is_fs_error(error, 'not_found')) {
			log?.error(st('yellow', `Failed to load package.json in ${dir}`), error);
		}
		return PACKAGE_JSON_EMPTY;
	}
	if (parse) {
		package_json = parse_package_json(PackageJson, package_json);
	} else if (typeof package_json !== 'object' || package_json === null) {
		return PACKAGE_JSON_EMPTY; // valid JSON that callers still read fields from
	}
	if (cache) {
		cache[dir] = package_json;
	}
	return package_json;
};

/**
 * Updates the `exports` of a SvelteKit library's `package.json` from the files in its lib directory,
 * then applies `map_package_json`, writing only when the contents change.
 * @param fs_paths - opens `package.json`, the lib directory, and the directories the paths name
 * @param map_package_json - maps the updated `package.json`, or returns `null` to leave it unchanged
 * @param log - logs the outcome
 * @param write - when `false`, reports whether it would change without writing
 * @param dir - the directory holding `package.json`, absolute or relative to the project;
 * defaults to the project of `fs_paths`
 * @param exports_dir - the directory whose files become the `exports`, relative to the project
 * (not to `dir`), defaults to `LIB_PATH`; the library check also reads the project's lib directory
 */
export const package_json_sync = async (
	fs_paths: FsPaths,
	map_package_json: PackageJsonMapper,
	log: Logger,
	write = true,
	dir: string = fs_paths.project.path,
	exports_dir?: string
): Promise<{ package_json: PackageJson | null; changed: boolean }> => {
	const updated = await package_json_update(
		fs_paths,
		async (package_json) => {
			if ((await has_sveltekit_library(fs_paths, package_json)).ok) {
				const exported_files = await fs_paths.search_path(exports_dir ?? LIB_PATH);
				package_json.exports = package_json_to_exports(exported_files.map((f) => f.path));
			}
			const mapped = await map_package_json(package_json);
			return mapped ? parse_package_json(PackageJson, mapped) : mapped;
		},
		dir,
		write
	);

	const exports_count =
		updated.changed && updated.package_json?.exports
			? Object.keys(updated.package_json.exports).length
			: 0;
	log.info(
		updated.changed
			? `updated package.json exports with ${exports_count} total export${plural(exports_count)}`
			: 'no changes to exports in package.json'
	);

	return updated;
};

const require = createRequire(import.meta.url);

/**
 * Loads the running Gro's own `package.json` through the module system,
 * which resolves it from this module, so it needs no fs capability.
 * @throws when it fails the `PackageJson` schema
 */
export const package_json_load_for_gro = (): PackageJson =>
	parse_package_json(PackageJson, require('@fuzdev/gro/package.json'));

/**
 * Reads the text of the `package.json` in `dir`.
 * @param fs_paths - opens the file
 * @param dir - absolute, or relative to the project
 * @throws `FsError` - when the file can't be read
 */
// TODO probably make this nullable and make callers handle failures
const package_json_load_contents = async (fs_paths: FsPaths, dir: string): Promise<string> => {
	const opened = await fs_paths.open_path(join(dir, PACKAGE_JSON_FILENAME));
	return opened.dir.read_text(opened.path);
};

/**
 * Writes the `package.json` in `dir`.
 * @param fs_paths - opens the file
 * @param serialized_package_json - the file's contents, as `package_json_serialize` makes them
 * @param dir - absolute, or relative to the project; defaults to the project of `fs_paths`
 */
export const package_json_write = async (
	fs_paths: FsPaths,
	serialized_package_json: string,
	dir: string = fs_paths.project.path
): Promise<void> => {
	const opened = await fs_paths.open_path(join(dir, PACKAGE_JSON_FILENAME));
	await opened.dir.write(opened.path, serialized_package_json);
};

export const package_json_serialize = (package_json: PackageJson): string =>
	JSON.stringify(parse_package_json(PackageJson, package_json), null, 2) + '\n';

/**
 * Updates the `package.json` in `dir`. Writes to the filesystem only when contents change.
 * @param fs_paths - opens the file
 * @param update - maps the loaded JSON, or returns `null` to leave it unchanged
 * @param dir - absolute, or relative to the project; defaults to the project of `fs_paths`
 * @param write - when `false`, reports whether it would change without writing
 */
export const package_json_update = async (
	fs_paths: FsPaths,
	update: (package_json: PackageJson) => PackageJson | null | Promise<PackageJson | null>,
	dir: string = fs_paths.project.path,
	write = true
): Promise<{ package_json: PackageJson | null; changed: boolean }> => {
	const original_contents = await package_json_load_contents(fs_paths, dir);
	const original = JSON.parse(original_contents);
	const updated = await update(original);
	if (updated === null) {
		return { package_json: original, changed: false };
	}
	const updated_contents = package_json_serialize(updated);
	if (updated_contents === original_contents) {
		return { package_json: original, changed: false };
	}
	if (write) await package_json_write(fs_paths, updated_contents, dir);
	return { package_json: updated, changed: true };
};

const is_index = (path: string): boolean => path === 'index.ts' || path === 'index.js';

/**
 * The `internal/` convention: modules under an `internal/` directory ship in
 * dist so public modules can import them, but they're not part of the public
 * surface. Each internal directory gets a `"./…/internal/*": null` exports
 * entry blocking consumer imports (Node's explicit-exclusion form — the null
 * key best-matches the directory's subpaths ahead of the broader wildcards;
 * exports keys allow a single `*`, hence one key per directory rather than an
 * any-depth pattern). Tooling honors the same signal: `svelte-docinfo` skips
 * null-blocked subpaths during exports discovery and excludes `internal/`
 * directories from analysis at any depth by default.
 *
 * Returns the blocking key for the path's outermost `internal` directory
 * segment (which covers any nested ones), or `null` for a public path — a
 * file merely *named* `internal` is public.
 */
const internal_export_key = (path: string): string | null => {
	const segments = path.split('/');
	const index = segments.indexOf('internal');
	if (index === -1 || index === segments.length - 1) return null;
	return './' + segments.slice(0, index + 1).join('/') + '/*';
};

export const package_json_to_exports = (paths: Array<string>): PackageJsonExports => {
	// wildcard flags come from the public files only — internal files can't
	// justify a wildcard whose internal subpaths the null keys then block
	const internal_keys: Set<string> = new Set();
	const public_paths: Array<string> = [];
	for (const path of paths) {
		const internal_key = internal_export_key(path);
		if (internal_key === null) {
			public_paths.push(path);
		} else {
			internal_keys.add(internal_key);
		}
	}
	const has_index = public_paths.some(is_index);
	const has_js = public_paths.some((p) => TS_MATCHER.test(p) || JS_MATCHER.test(p));
	const has_svelte = public_paths.some((p) => SVELTE_MATCHER.test(p));
	const has_json = public_paths.some((p) => JSON_MATCHER.test(p));
	const has_css = public_paths.some((p) => CSS_MATCHER.test(p));

	const exports: PackageJsonExports = {
		'./package.json': './package.json'
	};

	for (const internal_key of [...internal_keys].sort()) {
		exports[internal_key] = null;
	}

	if (has_index) {
		exports['.'] = {
			types: IMPORT_PREFIX + 'index.d.ts',
			default: IMPORT_PREFIX + 'index.js'
		};
	}

	if (has_js) {
		exports['./*.js'] = {
			types: IMPORT_PREFIX + '*.d.ts',
			default: IMPORT_PREFIX + '*.js'
		};
		exports['./*.ts'] = {
			types: IMPORT_PREFIX + '*.d.ts',
			default: IMPORT_PREFIX + '*.js'
		};
	}

	if (has_svelte) {
		exports['./*.svelte'] = {
			types: IMPORT_PREFIX + '*.svelte.d.ts',
			svelte: IMPORT_PREFIX + '*.svelte',
			default: IMPORT_PREFIX + '*.svelte'
		};
	}

	if (has_json) {
		exports['./*.json'] = {
			types: IMPORT_PREFIX + '*.json.d.ts',
			default: IMPORT_PREFIX + '*.json'
		};
	}

	if (has_css) {
		exports['./*.css'] = {
			default: IMPORT_PREFIX + '*.css'
		};
	}

	return parse_or_throw_formatted_error('package.json#exports', PackageJsonExports, exports);
};

const IMPORT_PREFIX = './' + SVELTEKIT_DIST_DIRNAME + '/';

export const package_json_parse_repo_url = (
	package_json: PackageJson
): { owner: string; repo: string } | undefined => {
	const { repository } = package_json;
	const repo_url = repository
		? typeof repository === 'string'
			? repository
			: repository.url
		: undefined;
	if (!repo_url) {
		return;
	}
	const parsed_repo_url = GITHUB_REPO_MATCHER.exec(strip_end(strip_end(repo_url, '/'), '.git'));
	if (!parsed_repo_url) {
		return;
	}
	const [, owner, repo] = parsed_repo_url;
	return { owner: owner!, repo: repo! };
};

/**
 * Parses a `PackageJson` object but preserves the order of the original keys.
 */
const parse_package_json = (schema: typeof PackageJson, value: any): PackageJson => {
	const parsed = parse_or_throw_formatted_error(PACKAGE_JSON_FILENAME, schema, value);
	const keys = Object.keys(value);
	return Object.fromEntries(
		Object.entries(parsed).sort(([a], [b]) => keys.indexOf(a) - keys.indexOf(b))
	) as any;
};

const parse_or_throw_formatted_error = <T extends s.AnySchema>(
	name: string,
	schema: T,
	value: unknown
): s.Infer<T> => {
	const parsed = s.parse(schema, value);
	if (!parsed.ok) {
		let msg = st('red', `Failed to parse ${name}:\n`);
		for (const issue of parsed.issues) {
			msg += st('red', `\n\t${s.format_issue(issue)}\n`);
		}
		throw Error(msg);
	}
	return parsed.value;
};

/**
 * @param include_peer - whether a `peerDependencies` entry counts. Pass `false` when
 * detecting what a project *is*, since a peer dep declares what it works alongside -
 * a plugin package peered on `@sveltejs/kit` isn't itself a SvelteKit app.
 */
export const package_json_has_dependency = (
	dep_name: string,
	package_json: PackageJson,
	include_peer = true
): boolean =>
	!!package_json.devDependencies?.[dep_name] ||
	!!package_json.dependencies?.[dep_name] ||
	(include_peer && !!package_json.peerDependencies?.[dep_name]);

export interface PackageJsonDep {
	name: string;
	version: string;
}

export const package_json_extract_dependencies = (
	package_json: PackageJson
): Array<PackageJsonDep> => {
	const deps_by_name: Map<string, PackageJsonDep> = new Map();
	// Earlier versions override later ones, so peer deps goes last.
	const add_deps = (deps: Record<string, string> | undefined) => {
		if (!deps) return;
		for (const [name, version] of Object.entries(deps)) {
			if (!deps_by_name.has(name)) {
				deps_by_name.set(name, { name, version });
			}
		}
	};
	add_deps(package_json.dependencies);
	add_deps(package_json.devDependencies);
	add_deps(package_json.peerDependencies);
	return Array.from(deps_by_name.values());
};
