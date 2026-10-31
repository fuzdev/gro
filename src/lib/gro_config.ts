import { join, resolve } from 'node:path';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { identity } from '@fuzdev/fuz_util/function.ts';
import type { PathFilter } from '@fuzdev/fuz_util/path.ts';
import { json_stringify_deterministic } from '@fuzdev/fuz_util/json.ts';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import { paths } from './paths.ts';
import {
	GRO_CONFIG_FILENAME,
	JS_CLI_DEFAULT,
	NODE_MODULES_DIRNAME,
	PM_CLI_DEFAULT,
	SERVER_DIST_PATH,
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_DIST_DIRNAME
} from './constants.ts';
import create_default_config from './gro.config.default.ts';
import type { PluginsCreateConfig } from './plugin.ts';
import type { PackageJsonMapper } from './package_json.ts';
import type { WatchOptions } from './module_graph.ts';

/**
 * BLAKE3 hash of empty string, used for configs without `build_cache_config`.
 * This ensures consistent cache behavior when no custom config is provided.
 */
export const EMPTY_BUILD_CACHE_CONFIG_HASH =
	'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262';

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
	 * When searching the filesystem for tasks, genfiles, and files to format,
	 * directories and files are included if they pass all of these filters.
	 * Each filter gets the path relative to the project root, like `src/lib/foo.ts`,
	 * whichever directory a search starts in, so the directories above the root
	 * never exclude anything; a path outside the root gets its absolute id.
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
	/**
	 * BLAKE3 hash of the user's `build_cache_config` from `gro.config.ts`, as deterministic JSON.
	 * This is computed during config normalization, which then drops the raw value from the config.
	 * If no `build_cache_config` was provided, this is the hash of an empty string.
	 * @see `RawGroConfig.build_cache_config`
	 */
	build_cache_config_hash: string;
	/**
	 * What the task context's `graph` and `watcher` watch.
	 * Defaults to `src/` with no filter.
	 */
	watch: WatchOptions;
}

/**
 * The relaxed variant of `GroConfig` that users can provide via `gro.config.ts`.
 * Superset of `GroConfig`.
 * @see https://github.com/fuzdev/gro/blob/main/src/docs/config.md
 */
export interface RawGroConfig {
	plugins?: PluginsCreateConfig;
	map_package_json?: PackageJsonMapper | null;
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
	 * Gro stores only an unkeyed hash of the value in `.gro/build.json` and never logs the raw value.
	 * The hash hides only high-entropy values: when every value in the object is short or
	 * guessable, such as flags and known URLs, hashing candidates recovers them.
	 * The process still holds the raw values it read, as in `process.env`.
	 */
	build_cache_config?:
		Record<string, unknown> | (() => Record<string, unknown> | Promise<Record<string, unknown>>);
	/**
	 * What the task context's `graph` and `watcher` watch, each field defaulting separately.
	 */
	watch?: Partial<WatchOptions>;
}

export type CreateGroConfig = (base_config: GroConfig) => RawGroConfig | Promise<RawGroConfig>;

export const create_empty_gro_config = (): GroConfig => ({
	plugins: () => [],
	map_package_json: identity,
	search_filters: [(id) => !SEARCH_EXCLUDER_DEFAULT.test(id)],
	js_cli: JS_CLI_DEFAULT,
	pm_cli: PM_CLI_DEFAULT,
	build_cache_config_hash: EMPTY_BUILD_CACHE_CONFIG_HASH,
	watch: { dir: paths.source, filter: null }
});

/**
 * The regexp used by default to exclude directories and files
 * when searching the filesystem for tasks and genfiles,
 * tested against paths relative to the project root.
 * Customize via `search_filters` in the `GroConfig`.
 * See the test cases for the exact behavior.
 */
export const SEARCH_EXCLUDER_DEFAULT = new RegExp(
	`(${
		'(^|/)\\.[^/]+' + // exclude all `.`-prefixed directories
		`|(^|/)${NODE_MODULES_DIRNAME}` +
		`|(^|/)${SVELTEKIT_BUILD_DIRNAME}` + // exclude the SvelteKit build directory
		`|(^|/)${SVELTEKIT_DIST_DIRNAME}` + // exclude the SvelteKit dist directory
		`|(^|/)${SERVER_DIST_PATH}` // exclude the Gro server plugin dist directory
	})($|/)`,
	'u'
);

/**
 * Transforms a `RawGroConfig` to the more strict `GroConfig`.
 * This allows users to provide a more relaxed config.
 * Hashes the `build_cache_config` and drops the raw value from the config.
 */
export const cook_gro_config = async (raw_config: RawGroConfig): Promise<GroConfig> => {
	const empty_config = create_empty_gro_config();

	// All of the raw config properties are optional,
	// so fall back to the empty values when `undefined`.
	const {
		plugins = empty_config.plugins,
		map_package_json = empty_config.map_package_json,
		search_filters = empty_config.search_filters,
		js_cli = empty_config.js_cli,
		pm_cli = empty_config.pm_cli,
		build_cache_config,
		watch
	} = raw_config;

	// hash build_cache_config and drop the raw value, so it isn't passed around with the config
	let build_cache_config_hash: string;
	if (!build_cache_config) {
		build_cache_config_hash = EMPTY_BUILD_CACHE_CONFIG_HASH;
	} else {
		// Resolve if it's a function
		const resolved =
			typeof build_cache_config === 'function' ? await build_cache_config() : build_cache_config;

		// Hash the JSON representation with deterministic key ordering
		build_cache_config_hash = hash_blake3(json_stringify_deterministic(resolved));
	}

	// `delete` drops the reference, it doesn't scrub memory
	delete (raw_config as any).build_cache_config;

	return {
		plugins,
		map_package_json,
		search_filters: Array.isArray(search_filters)
			? search_filters
			: search_filters
				? [search_filters]
				: [],
		js_cli,
		pm_cli,
		build_cache_config_hash,
		watch: {
			dir: resolve(watch?.dir ?? empty_config.watch.dir),
			filter: watch?.filter ?? empty_config.watch.filter
		}
	};
};

export interface GroConfigModule {
	readonly default: RawGroConfig | CreateGroConfig;
}

export const load_gro_config = async (dir = paths.root): Promise<GroConfig> => {
	const default_config = await cook_gro_config(
		await create_default_config(create_empty_gro_config())
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
			: config_module.default
	);
};

export const validate_gro_config_module: (
	config_module: any,
	config_path: string
) => asserts config_module is GroConfigModule = (config_module, config_path) => {
	const config = config_module.default;
	if (!config) {
		throw Error(`Invalid Gro config module at ${config_path}: expected a default export`);
	} else if (!(typeof config === 'function' || typeof config === 'object')) {
		throw Error(
			`Invalid Gro config module at ${config_path}: the default export must be a function or object`
		);
	}
};
