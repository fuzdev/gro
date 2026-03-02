import {join, resolve} from 'node:path';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import {identity} from '@fuzdev/fuz_util/function.js';
import type {PathFilter, PathId} from '@fuzdev/fuz_util/path.js';
import {json_stringify_deterministic} from '@fuzdev/fuz_util/json.js';
import {hash_secure} from '@fuzdev/fuz_util/hash.js';

import {GRO_DIST_DIR, IS_THIS_GRO, paths} from './paths.ts';
import {
	GRO_CONFIG_FILENAME,
	JS_CLI_DEFAULT,
	NODE_MODULES_DIRNAME,
	PM_CLI_DEFAULT,
	SERVER_DIST_PATH,
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_DIST_DIRNAME,
} from './constants.ts';
import create_default_config from './gro.config.default.ts';
import type {PluginsCreateConfig} from './plugin.ts';
import type {PackageJsonMapper} from './package_json.ts';
import type {ParsedSvelteConfig} from './svelte_config.ts';
import type {FilerOptions} from './filer.ts';

/**
 * SHA-256 hash of empty string, used for configs without build_cache_config.
 * This ensures consistent cache behavior when no custom config is provided.
 */
export const EMPTY_BUILD_CACHE_CONFIG_HASH =
	'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

/**
 * The config that users can extend via `gro.config.ts`.
 * This is exposed to users in places like tasks and genfiles.
 * @see https://github.com/fuzdev/gro/blob/main/src/docs/config.md
 */
export interface GroConfig extends RawGroConfig {
	/**
	 * @see https://github.com/fuzdev/gro/blob/main/src/docs/plugin.md
	 */
	plugins: PluginsCreateConfig;
	/**
	 * Maps the project's `package.json` before writing it to the filesystem.
	 * The `package_json` argument may be mutated, but the return value is what's used by the caller.
	 * Returning `null` is a no-op for the caller.
	 */
	map_package_json: PackageJsonMapper | null;
	/**
	 * The root directories to search for tasks given implicit relative input paths.
	 * Defaults to `./src/lib`, then the cwd, then the Gro package dist.
	 */
	task_root_dirs: Array<PathId>;
	/**
	 * When searching the filesystem for tasks and genfiles,
	 * directories and files are included if they pass all of these filters.
	 */
	search_filters: Array<PathFilter>;
	/**
	 * The CLI to use that's compatible with `node`.
	 */
	js_cli: string;
	/**
	 * The CLI to use that's compatible with `npm install` and `npm link`. Defaults to `'npm'`.
	 */
	pm_cli: string;
	/** @default SVELTE_CONFIG_FILENAME */
	svelte_config_filename?: string;
	/**
	 * SHA-256 hash of the user's `build_cache_config` from `gro.config.ts`.
	 * This is computed during config normalization and the raw value is immediately deleted.
	 * If no `build_cache_config` was provided, this is the hash of an empty string.
	 * @see RawGroConfig.build_cache_config
	 */
	build_cache_config_hash: string;
	/**
	 * Options passed to the `Filer` for file watching and import resolution.
	 * @see FilerOptions
	 */
	filer_options: Partial<FilerOptions> | null;
}

/**
 * The relaxed variant of `GroConfig` that users can provide via `gro.config.ts`.
 * Superset of `GroConfig`.
 * @see https://github.com/fuzdev/gro/blob/main/src/docs/config.md
 */
export interface RawGroConfig {
	plugins?: PluginsCreateConfig;
	map_package_json?: PackageJsonMapper | null;
	task_root_dirs?: Array<string>;
	search_filters?: PathFilter | Array<PathFilter> | null;
	js_cli?: string;
	pm_cli?: string;
	/**
	 * Optional object defining custom build inputs for cache invalidation.
	 * This value is hashed during config normalization and used to detect
	 * when builds need to be regenerated due to non-source changes.
	 *
	 * Use cases:
	 * - Environment variables baked into build: `{api_url: process.env.PUBLIC_API_URL}`
	 * - External data files: `{data: fs.readFileSync('data.json', 'utf-8')}`
	 * - Build feature flags: `{enable_analytics: true}`
	 *
	 * Can be a static object or an async function that returns an object.
	 *
	 * IMPORTANT: It's safe to include secrets here because they are hashed and `delete`d
	 * during config normalization. The raw value is never logged or persisted.
	 */
	build_cache_config?:
		| Record<string, unknown>
		| (() => Record<string, unknown> | Promise<Record<string, unknown>>);
	/**
	 * Options passed to the `Filer` for file watching and import resolution.
	 * @see FilerOptions
	 */
	filer_options?: Partial<FilerOptions> | null;
}

export type CreateGroConfig = (
	base_config: GroConfig,
	svelte_config?: ParsedSvelteConfig,
) => RawGroConfig | Promise<RawGroConfig>;

export const create_empty_gro_config = (): GroConfig => ({
	plugins: () => [],
	map_package_json: identity,
	task_root_dirs: [
		// TODO maybe disable if no SvelteKit `lib` directory? or other detection to improve defaults
		paths.lib,
		IS_THIS_GRO ? null : paths.root,
		IS_THIS_GRO ? null : GRO_DIST_DIR,
	].filter((v) => v !== null),
	search_filters: [(id) => !SEARCH_EXCLUDER_DEFAULT.test(id)],
	js_cli: JS_CLI_DEFAULT,
	pm_cli: PM_CLI_DEFAULT,
	build_cache_config_hash: EMPTY_BUILD_CACHE_CONFIG_HASH,
	filer_options: null,
});

/**
 * The regexp used by default to exclude directories and files
 * when searching the filesystem for tasks and genfiles.
 * Customize via `search_filters` in the `GroConfig`.
 * See the test cases for the exact behavior.
 */
export const SEARCH_EXCLUDER_DEFAULT = new RegExp(
	`(${
		'(^|/)\\.[^/]+' + // exclude all `.`-prefixed directories
		// TODO probably change to `pkg.name` instead of this catch-all (also `gro` below)
		`|(^|/)${NODE_MODULES_DIRNAME}(?!/(@[^/]+/)?gro/${SVELTEKIT_DIST_DIRNAME})` + // exclude `node_modules` unless it's to the Gro directory
		`|(^|/)${SVELTEKIT_BUILD_DIRNAME}` + // exclude the SvelteKit build directory
		`|(^|/)(?<!(^|/)gro/)${SVELTEKIT_DIST_DIRNAME}` + // exclude the SvelteKit dist directory unless it's in the Gro directory
		`|(^|/)${SERVER_DIST_PATH}` // exclude the Gro server plugin dist directory
	})($|/)`,
	'u',
);

/**
 * Transforms a `RawGroConfig` to the more strict `GroConfig`.
 * This allows users to provide a more relaxed config.
 * Hashes the `build_cache_config` and deletes the raw value for security.
 */
export const cook_gro_config = async (raw_config: RawGroConfig): Promise<GroConfig> => {
	const empty_config = create_empty_gro_config();

	// All of the raw config properties are optional,
	// so fall back to the empty values when `undefined`.
	const {
		plugins = empty_config.plugins,
		map_package_json = empty_config.map_package_json,
		task_root_dirs = empty_config.task_root_dirs,
		search_filters = empty_config.search_filters,
		js_cli = empty_config.js_cli,
		pm_cli = empty_config.pm_cli,
		build_cache_config,
		filer_options = empty_config.filer_options,
	} = raw_config;

	// Hash build_cache_config and delete the raw value
	// IMPORTANT: Raw value may contain secrets - hash it and delete immediately
	let build_cache_config_hash: string;
	if (!build_cache_config) {
		build_cache_config_hash = EMPTY_BUILD_CACHE_CONFIG_HASH;
	} else {
		// Resolve if it's a function
		const resolved =
			typeof build_cache_config === 'function' ? await build_cache_config() : build_cache_config;

		// Hash the JSON representation with deterministic key ordering
		build_cache_config_hash = await hash_secure(json_stringify_deterministic(resolved));
	}

	// Delete the raw value to ensure it doesn't persist in memory
	delete (raw_config as any).build_cache_config;

	return {
		plugins,
		map_package_json,
		task_root_dirs: task_root_dirs.map((p) => resolve(p)),
		search_filters: Array.isArray(search_filters)
			? search_filters
			: search_filters
				? [search_filters]
				: [],
		js_cli,
		pm_cli,
		build_cache_config_hash,
		filer_options: filer_options ?? null,
	};
};

export interface GroConfigModule {
	readonly default: RawGroConfig | CreateGroConfig;
}

export const load_gro_config = async (dir = paths.root): Promise<GroConfig> => {
	const default_config = await cook_gro_config(
		await create_default_config(create_empty_gro_config()),
	);

	const config_path = join(dir, GRO_CONFIG_FILENAME);
	if (!(await fs_exists(config_path))) {
		// No user config file found, so return the default.
		return default_config;
	}

	// Import the user's `gro.config.ts`.
	const config_module = await import(config_path);

	validate_gro_config_module(config_module, config_path);

	return await cook_gro_config(
		typeof config_module.default === 'function'
			? await config_module.default(default_config)
			: config_module.default,
	);
};

export const validate_gro_config_module: (
	config_module: any,
	config_path: string,
) => asserts config_module is GroConfigModule = (config_module, config_path) => {
	const config = config_module.default;
	if (!config) {
		throw Error(`Invalid Gro config module at ${config_path}: expected a default export`);
	} else if (!(typeof config === 'function' || typeof config === 'object')) {
		throw Error(
			`Invalid Gro config module at ${config_path}: the default export must be a function or object`,
		);
	}
};
