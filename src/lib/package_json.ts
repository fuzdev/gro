import {z} from 'zod';
import {join} from 'node:path';
import {readFile, writeFile} from 'node:fs/promises';
import {plural, strip_end} from '@fuzdev/fuz_util/string.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import {styleText as st} from 'node:util';
import {PackageJson, PackageJsonExports} from '@fuzdev/fuz_util/package_json.js';
import {fs_search} from '@fuzdev/fuz_util/fs.js';

import {paths, gro_paths, IS_THIS_GRO} from './paths.ts';
import {
	PACKAGE_JSON_FILENAME,
	SVELTEKIT_DIST_DIRNAME,
	TS_MATCHER,
	JS_MATCHER,
	SVELTE_MATCHER,
	JSON_MATCHER,
	CSS_MATCHER,
} from './constants.ts';
import {has_sveltekit_library} from './sveltekit_helpers.ts';
import {GITHUB_REPO_MATCHER} from './github.ts';

export type PackageJsonMapper = (
	package_json: PackageJson,
) => PackageJson | null | Promise<PackageJson | null>;

export const PACKAGE_JSON_EMPTY: PackageJson = {name: '', version: ''};

export const package_json_load = async (
	dir = IS_THIS_GRO ? gro_paths.root : paths.root,
	cache?: Record<string, PackageJson>,
	parse = true, // TODO pass `false` here in more places, especially anything perf-sensitive like work on startup
	log?: Logger,
): Promise<PackageJson> => {
	let package_json: PackageJson;
	if (cache && dir in cache) {
		return cache[dir]!;
	}
	try {
		package_json = JSON.parse(await package_json_load_contents(dir));
	} catch (error) {
		log?.error(st('yellow', `Failed to load package.json in ${dir}`), error);
		return PACKAGE_JSON_EMPTY;
	}
	if (parse) {
		package_json = parse_package_json(PackageJson, package_json);
	}
	if (cache) {
		cache[dir] = package_json;
	}
	return package_json;
};

export const package_json_sync = async (
	map_package_json: PackageJsonMapper,
	log: Logger,
	write = true,
	dir = paths.root,
	exports_dir = paths.lib,
): Promise<{package_json: PackageJson | null; changed: boolean}> => {
	const exported_files = await fs_search(exports_dir);
	const exported_paths = exported_files.map((f) => f.path);
	const updated = await package_json_update(
		async (package_json) => {
			if ((await has_sveltekit_library(package_json)).ok) {
				package_json.exports = package_json_to_exports(exported_paths);
			}
			const mapped = await map_package_json(package_json);
			return mapped ? parse_package_json(PackageJson, mapped) : mapped;
		},
		dir,
		write,
	);

	const exports_count =
		updated.changed && updated.package_json?.exports
			? Object.keys(updated.package_json.exports).length
			: 0;
	log.info(
		updated.changed
			? `updated package.json exports with ${exports_count} total export${plural(exports_count)}`
			: 'no changes to exports in package.json',
	);

	return updated;
};

export const package_json_load_for_gro = (): Promise<PackageJson> =>
	package_json_load(gro_paths.root);

// TODO probably make this nullable and make callers handle failures
const package_json_load_contents = (dir: string): Promise<string> =>
	readFile(join(dir, PACKAGE_JSON_FILENAME), 'utf8');

export const package_json_write = (serialized_package_json: string): Promise<void> =>
	writeFile(join(paths.root, PACKAGE_JSON_FILENAME), serialized_package_json);

export const package_json_serialize = (package_json: PackageJson): string =>
	JSON.stringify(parse_package_json(PackageJson, package_json), null, 2) + '\n';

/**
 * Updates `package.json`. Writes to the filesystem only when contents change.
 */
export const package_json_update = async (
	update: (package_json: PackageJson) => PackageJson | null | Promise<PackageJson | null>,
	dir = paths.root,
	write = true,
): Promise<{package_json: PackageJson | null; changed: boolean}> => {
	const original_contents = await package_json_load_contents(dir);
	const original = JSON.parse(original_contents);
	const updated = await update(original);
	if (updated === null) {
		return {package_json: original, changed: false};
	}
	const updated_contents = package_json_serialize(updated);
	if (updated_contents === original_contents) {
		return {package_json: original, changed: false};
	}
	if (write) await package_json_write(updated_contents);
	return {package_json: updated, changed: true};
};

const is_index = (path: string): boolean => path === 'index.ts' || path === 'index.js';

export const package_json_to_exports = (paths: Array<string>): PackageJsonExports => {
	const has_index = paths.some(is_index);
	const has_js = paths.some((p) => TS_MATCHER.test(p) || JS_MATCHER.test(p));
	const has_svelte = paths.some((p) => SVELTE_MATCHER.test(p));
	const has_json = paths.some((p) => JSON_MATCHER.test(p));
	const has_css = paths.some((p) => CSS_MATCHER.test(p));

	const exports: PackageJsonExports = {
		'./package.json': './package.json',
	};

	if (has_index) {
		exports['.'] = {
			types: IMPORT_PREFIX + 'index.d.ts',
			default: IMPORT_PREFIX + 'index.js',
		};
	}

	if (has_js) {
		exports['./*.js'] = {
			types: IMPORT_PREFIX + '*.d.ts',
			default: IMPORT_PREFIX + '*.js',
		};
		exports['./*.ts'] = {
			types: IMPORT_PREFIX + '*.d.ts',
			default: IMPORT_PREFIX + '*.js',
		};
	}

	if (has_svelte) {
		exports['./*.svelte'] = {
			types: IMPORT_PREFIX + '*.svelte.d.ts',
			svelte: IMPORT_PREFIX + '*.svelte',
			default: IMPORT_PREFIX + '*.svelte',
		};
	}

	if (has_json) {
		exports['./*.json'] = {
			types: IMPORT_PREFIX + '*.json.d.ts',
			default: IMPORT_PREFIX + '*.json',
		};
	}

	if (has_css) {
		exports['./*.css'] = {
			default: IMPORT_PREFIX + '*.css',
		};
	}

	return parse_or_throw_formatted_error('package.json#exports', PackageJsonExports, exports);
};

const IMPORT_PREFIX = './' + SVELTEKIT_DIST_DIRNAME + '/';

export const package_json_parse_repo_url = (
	package_json: PackageJson,
): {owner: string; repo: string} | undefined => {
	const {repository} = package_json;
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
	return {owner: owner!, repo: repo!};
};

/**
 * Parses a `PackageJson` object but preserves the order of the original keys.
 */
const parse_package_json = (schema: typeof PackageJson, value: any): PackageJson => {
	const parsed = parse_or_throw_formatted_error(PACKAGE_JSON_FILENAME, schema, value);
	const keys = Object.keys(value);
	return Object.fromEntries(
		Object.entries(parsed).sort(([a], [b]) => keys.indexOf(a) - keys.indexOf(b)),
	) as any;
};

// TODO maybe extract to zod helpers? see also everything in `task_logging.ts`
const parse_or_throw_formatted_error = <T extends z.ZodType>(
	name: string,
	schema: T,
	value: any,
): z.infer<T> => {
	const parsed = schema.safeParse(value);
	if (!parsed.success) {
		let msg = st('red', `Failed to parse ${name}:\n`);
		for (const issue of parsed.error.issues) {
			msg += st('red', `\n\t"${issue.path}" ${issue.message}\n`);
		}
		throw Error(msg);
	}
	return parsed.data;
};

export const package_json_has_dependency = (dep_name: string, package_json: PackageJson): boolean =>
	!!package_json.devDependencies?.[dep_name] ||
	!!package_json.dependencies?.[dep_name] ||
	!!package_json.peerDependencies?.[dep_name];

export interface PackageJsonDep {
	name: string;
	version: string;
}

export const package_json_extract_dependencies = (
	package_json: PackageJson,
): Array<PackageJsonDep> => {
	const deps_by_name: Map<string, PackageJsonDep> = new Map();
	// Earlier versions override later ones, so peer deps goes last.
	const add_deps = (deps: Record<string, string> | undefined) => {
		if (!deps) return;
		for (const [name, version] of Object.entries(deps)) {
			if (!deps_by_name.has(name)) {
				deps_by_name.set(name, {name, version});
			}
		}
	};
	add_deps(package_json.dependencies);
	add_deps(package_json.devDependencies);
	add_deps(package_json.peerDependencies);
	return Array.from(deps_by_name.values());
};
