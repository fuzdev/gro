# config

Gro supports SvelteKit apps, Node libraries, and Node servers with minimal abstraction
with the help of an optional config file that lives at the root `gro.config.ts`.
If a project does not define a config, Gro imports a default config from
[`src/lib/gro.config.default.ts`](/src/lib/gro.config.default.ts),
which looks at your project for the familiar patterns and tries to do the right thing,
without required deps.

> The [default config](/src/lib/gro.config.default.ts)
> detects three types of projects that can coexist in one repo:
> SvelteKit frontends,
> Node libraries with [`@sveltejs/package`](https://svelte.dev/docs/kit/packaging),
> and Node servers.

See [`src/lib/gro_config.ts`](/src/lib/gro_config.ts) for the config types and implementation.

## examples

[The default config](/src/lib/gro.config.default.ts)
is used for projects that do not define `gro.config.ts`.
It's also passed as the first argument to `CreateGroConfig`.

A simple config that does nothing:

```ts
// gro.config.ts
import type {CreateGroConfig} from '@fuzdev/gro';

const config: CreateGroConfig = async (cfg) => {
	// mutate `cfg` or return a new object
	return cfg;
};

export default config;
```

The default export of a Gro config is `GroConfig | CreateGroConfig`:

```ts
export interface CreateGroConfig {
	(base_config: GroConfig): RawGroConfig | Promise<RawGroConfig>;
}

// The strict variant that's used internally and exposed to users in tasks and elsewhere.
export interface GroConfig extends RawGroConfig {
	plugins: PluginsCreateConfig;
	map_package_json: MapPackageJson | null;
	task_root_dirs: Array<PathId>;
	search_filters: Array<PathFilter>;
	js_cli: string;
	pm_cli: string;
}

// The relaxed variant that users can provide. Superset of `GroConfig`.
export interface RawGroConfig {
	plugins?: PluginsCreateConfig;
	map_package_json?: MapPackageJson | null;
	task_root_dirs?: Array<string>;
	search_filters?: PathFilter | Array<PathFilter> | null;
	js_cli?: string;
	pm_cli?: string;
}
```

To define a user config that overrides the default plugins:

```ts
import type {CreateGroConfig} from '@fuzdev/gro';
import {gro_plugin_sveltekit_app} from '@fuzdev/gro/gro_plugin_sveltekit_app.js';

const config: CreateGroConfig = async (cfg) => {
	// `cfg`, which has type `GroConfig` and is equal to `create_empty_gro_config()`,
	// can be mutated or you can return your own.
	// A return value is required to avoid potential errors and reduce ambiguity.

	// example setting your own plugins):
	cfg.plugins = async () => [
		gro_plugin_sveltekit_app(),
		(await import('./src/custom_plugin.js')).plugin(),
	];

	// example extending the default plugins:
	const get_base_plugins = cfg.plugins;
	cfg.plugins = async (ctx) => {
		// replace a base plugin with `import {plugin_replace} from '@fuzdev/gro';`:
		const updated_plugins = plugin_replace(
			await get_base_plugins(ctx),
			gro_plugin_sveltekit_app(),
			// 'gro_plugin_sveltekit_app', // optional name if they don't match
		);
		return updated_plugins.concat(create_some_custom_plugin());
		// `return get_base_plugins(ctx)` is the base behavior
	};

	return cfg; // return type is `RawGroConfig`, which is a relaxed superset of `GroConfig`
};

export default config;
```

You can also export a config object and use `create_empty_gro_config` to get the defaults:

```ts
import {create_empty_gro_config} from '@fuzdev/gro/gro_config.js';

const config = create_empty_gro_config();

// config.plugins = ...;
// config.map_package_json = ...;
// config.task_root_dirs = ...;
// config.search_filters = ...;
// config.js_cli = ...;
// config.pm_cli = ...;

export default config;
```

See also [Gro's own internal config](/gro.config.ts).

## `plugins`

The `plugins` property is a function that returns an array of `Plugin` instances.
Read more about plugins and the `Plugin` in
[plugin.md](plugin.md), [dev.md](dev.md#plugin), and [build.md](build.md#plugins).

```ts
export type PluginsCreateConfig<TPluginContext extends PluginContext = PluginContext> = (
	ctx: TPluginContext,
) => Array<Plugin<TPluginContext>> | Promise<Array<Plugin<TPluginContext>>>;
```

## `map_package_json`

The Gro config option `map_package_json` hooks into Gro's `package.json` automations.
The `gro sync` task, which is called during the dev and build tasks among others,
performs several steps to get a project's state ready,
including `svelte-kit sync` and `package.json` automations.

When the `map_package_json` config value is truthy,
`gro sync` writes to the root `package.json` on the filesystem with a mapped version.
To opt out, configure `map_package_json` to `null` or return `null` from it.

> The `gro check` task integrates with `map_package_json` to ensure everything is synced.

The main purpose of `map_package_json` is to automate
the `"exports"` property of your root `package.json`.
The motivation is to streamline package publishing by supplementing
[`@sveltejs/package`](https://svelte.dev/docs/kit/packaging).

By default `package_json.exports` uses subpath wildcard patterns to include everything from `$lib/`
except for some ignored files like tests and markdown,
and you can provide your own `map_package_json` hook to
mutate and return the `package_json`, return a new one,
or return `null` to opt out of transforming it completely.

### using `map_package_json`

```ts
// gro.config.ts
const config: GroConfig = {
	// ...other config

	// default behavior is the identity function:
	map_package_json: (p) => p,

	// disable mapping `package.json` with automated `exports`:
	map_package_json: null,

	// mutate anything and return the final config (can be async):
	map_package_json: (package_json) => {
		// example setting `exports`:
		package_json.exports = {
			'.': {
				default: './dist/index.js',
				types: './dist/index.d.ts',
			},
			'./example.js': {
				default: './dist/example.js',
				types: './dist/example.d.ts',
			},
			'./Example.svelte': {
				svelte: './dist/Example.svelte',
				types: './dist/Example.svelte.d.ts',
			},
		};
		// example filtering `exports`:
		package_json.exports = Object.fromEntries(
			Object.entries(package_json.exports).filter(/* ... */),
		);
		return package_json; // returning `null` is a no-op
	},
};

export type PackageJsonMapper = (
	package_json: PackageJson,
) => PackageJson | null | Promise<PackageJson | null>;
```

## `task_root_dirs`

The Gro config option `task_root_dirs` allows customizing Gro's task resolution.
When calling `gro [input_path]`, absolute and explicitly relative paths starting with `.`
are resolved according to normal filesystem rules,
but non-explicit input paths, like `foo`, are resolved by searching
through `task_root_dirs` in order until a matching file or directory is found on the filesystem.

The default task paths are `./src/lib`, then `.`, and then Gro's dist directory.

## `search_filters`

The Gro config option `search_filters` allows customizing
how Gro searches for tasks and genfiles on the filesystem.
Directories and files are included if they pass all of these filters.

By default, it uses the `SEARCH_EXCLUDER_DEFAULT` to exclude
dot-prefixed directories, node_modules,
and the build and dist directories for SvelteKit and Gro.

## `js_cli`

> ⚠️ support is currently partial, help is welcome

The CLI to use that's compatible with `node`.

## `pm_cli`

> ⚠️ support is currently partial, help is welcome

The CLI to use that's compatible with `npm install` and `npm link`.
Defaults to `'npm'`.

## `build_cache_config`

The `build_cache_config` option defines custom build inputs
that invalidate the [build cache](build.md#build-caching) when they change.
Gro's build cache uses git commit hash to detect when code, dependencies, or
configs change, and only works with a clean workspace (see [dirty workspace behavior](build.md#dirty-workspace-behavior)).
Use `build_cache_config` when your build also depends on external
factors like environment variables, remote data, or feature flags.

This value is hashed before being stored in the cache metadata.
The raw value is never logged or written to disk, protecting sensitive information.

### when to use `build_cache_config`

The build cache automatically invalidates on any git commit (source code, dependencies,
configs -- assuming you commit changes before building in the normal case)
Use `build_cache_config` when your build also depends on:

- environment variables baked into the build (API endpoints, feature flags)
- external data files that affect the build (content databases, configuration data)
- runtime feature flags that change build behavior
- build-time constants from non-standard sources

### basic usage

```ts
// gro.config.ts
import type {GroConfig} from '@fuzdev/gro';
import {readFileSync} from 'node:fs';

export default {
	build_cache_config: {
		// Environment variables that affect the build
		// It's safe to include secrets here because they are hashed and `delete`d
		api_endpoint: process.env.PUBLIC_API_URL,
		analytics_key: process.env.PUBLIC_ANALYTICS_KEY,

		// Runtime information (if build outputs vary by platform/arch)
		platform: process.platform,
		arch: process.arch,

		// External data that influences the build
		data_version: readFileSync('data/version.txt', 'utf-8'),

		// Feature flags
		features: {
			enable_analytics: true,
			enable_beta_ui: false,
		},
	},
} satisfies GroConfig;
```

Any change to these values will trigger a fresh build, even if source code hasn't changed.

### async function usage

For complex scenarios, you can provide an async function:

```ts
// gro.config.ts
export default {
	build_cache_config: async () => {
		const config_data = await fetch('https://api.fuz.dev/build-config').then((r) => r.json());

		return {
			remote_config_version: config_data.version,
			feature_flags: config_data.flags,
		};
	},
} satisfies GroConfig;
```

### security considerations

The `build_cache_config` value is hashed before being written to `.gro/build.json`.
Only the hash is stored, never the raw values, so it's safe to include:

- API keys (though using them at build time should be carefully considered)
- Internal URLs
- Configuration secrets

However, be aware that these values may still appear in:

- Build outputs if your code embeds them
- Build logs if explicitly logged elsewhere
- The config file itself (ensure `gro.config.ts` is not publicly committed if it contains secrets)

### how it works

The `build_cache_config` is hashed early during config normalization (in `load_gro_config()`).
This protects sensitive values by ensuring the raw config never persists in memory.

During the build:

1. Checks if workspace has uncommitted changes (if dirty, skips cache entirely)
2. Loads previous build's cache metadata from `.gro/build.json` (missing/corrupt = rebuild)
3. Compares current git commit against cached commit (different = rebuild)
4. Compares current config hash against cached config hash (different = rebuild)
5. If cache key matches, validates output files haven't been modified
6. If all checks pass, uses the cached build

This ensures builds are correct while protecting sensitive configuration.
Both cache factors (git commit and `build_cache_config` hash) are checked—if either changes,
the cache is invalidated.

### common patterns

```ts
// platform-specific builds
build_cache_config: {
  node: process.version,
  platform: process.platform,
  arch: process.arch,
}

// feature flags from environment
build_cache_config: () => ({
  features: Object.fromEntries(
    Object.entries(process.env)
      .filter(([k]) => k.startsWith('FEATURE_'))
  ),
})

// external data version
build_cache_config: async () => ({
  data_version: await fs.promises.readFile('data/version.txt', 'utf-8'),
  // hash large files instead of including content
  schema_hash: hash_blake3(await fs.promises.readFile('schema.sql')),
})
```

> ⚠️ keep resolution fast - async functions are called on every build
