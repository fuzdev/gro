# config

Gro supports SvelteKit apps, Node libraries, and Node servers with minimal abstraction
with the help of an optional config file that lives at the root `gro.config.ts`.
If a project does not define a config, Gro imports a default config from
[`src/lib/gro.config.default.ts`](/src/lib/gro.config.default.ts),
which looks at your project for the familiar patterns and tries to do the right thing,
without required deps.

> Where it's going: config stays a TypeScript module.
> On the [runtime](runtime.md) it runs confined like any other,
> and `gro eval` gives any tool a confined, deterministic one-shot eval of a module like this one.

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
	search_filters: Array<PathFilter>;
	js_cli: string;
	pm_cli: string;
	watch: WatchOptions;
}

// The relaxed variant that users can provide. Superset of `GroConfig`.
export interface RawGroConfig {
	plugins?: PluginsCreateConfig;
	map_package_json?: MapPackageJson | null;
	search_filters?: PathFilter | Array<PathFilter> | null;
	js_cli?: string;
	pm_cli?: string;
	watch?: Partial<WatchOptions>;
}
```

To define a user config that overrides the default plugins:

```ts
import type {CreateGroConfig} from '@fuzdev/gro';
import {gro_plugin_sveltekit_app} from '@fuzdev/gro/gro_plugin_sveltekit_app.ts';

const config: CreateGroConfig = async (cfg) => {
	// `cfg`, which has type `GroConfig` and is equal to `create_empty_gro_config()`,
	// can be mutated or you can return your own.
	// A return value is required to avoid potential errors and reduce ambiguity.

	// example setting your own plugins):
	cfg.plugins = async () => [
		gro_plugin_sveltekit_app(),
		(await import('./src/custom_plugin.ts')).plugin(),
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
import {create_empty_gro_config} from '@fuzdev/gro/gro_config.ts';

const config = create_empty_gro_config();

// config.plugins = ...;
// config.map_package_json = ...;
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

By default `package_json.exports` uses subpath wildcard patterns to include everything from `src/lib/`
except for some ignored files like tests and markdown.
Every `internal/` directory is blocked at any depth by a null exports entry
(`"./internal/*": null`, one per outermost internal directory),
so internal modules ship in dist for public modules to import
but can't be imported by consumers,
and internal files don't count toward which wildcard patterns are emitted.
You can provide your own `map_package_json` hook to
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

## `search_filters`

The Gro config option `search_filters` allows customizing
how Gro searches for tasks, genfiles, and files to format on the filesystem.
Directories and files are included if they pass all of these filters,
each given the path relative to the project root, like `src/lib/foo.ts`,
whichever directory a search starts in, or its absolute id outside the root.

By default, it uses the `SEARCH_EXCLUDER_DEFAULT` to exclude
dot-prefixed directories, node_modules,
and the build and dist directories for SvelteKit and Gro.

## `watch`

What the task context's `graph` and `watcher` watch:
`dir`, the directory holding the project modules (default `src/`),
and `filter`, a `PathFilter` excluding the paths it rejects (default `null`).
Files outside `dir` enter the graph only when a project module imports them.

`filter` is called with absolute paths,
unlike [`search_filters`](#search_filters), which test paths relative to the project root.
It filters change events, not the directory walk:
a directory it rejects is still watched, and each path below it is filtered on its own.

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
Gro's build cache uses the git commit hash to detect when committed code or configs change,
plus the Gro and Node versions and npm's record of the installed packages
(see [cache key](build.md#cache-key)),
and only works with a clean workspace (see [dirty workspace behavior](build.md#dirty-workspace-behavior)).
Use `build_cache_config` when your build also depends on external
factors like environment variables, remote data, or feature flags.

This value is hashed before being stored in the cache metadata,
and the raw value is never logged or written to disk.
The hash hides only high-entropy values (see [security considerations](#security-considerations)).

### when to use `build_cache_config`

The build cache automatically invalidates on any git commit (source code, `package.json`
and the lockfile, configs -- assuming you commit changes before building in the normal case),
on a Gro or Node upgrade, and when npm changes what's installed.
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
Only the hash is stored, never the raw values.
The hash is an unkeyed BLAKE3 of the object's deterministic JSON, so it hides only
high-entropy values like random API keys. When every value is short or guessable,
like flags, versions, and known URLs, anyone who can read `.gro/build.json`
can recover them by hashing candidates.

Be aware that these values may also appear in:

- Build outputs if your code embeds them
- Build logs if explicitly logged elsewhere
- The config file itself (ensure `gro.config.ts` is not publicly committed if it contains secrets)
- The Gro process, which keeps whatever it read, like `process.env`

### how it works

The `build_cache_config` is hashed early during config normalization (in `load_gro_config()`),
which then drops the raw value from the config, so plugins and tasks see only the hash.

During the build:

1. Checks if workspace has uncommitted changes (if dirty, skips cache entirely)
2. Loads previous build's cache metadata from `.gro/build.json` (missing/corrupt = rebuild)
3. Compares current git commit against cached commit (different = rebuild)
4. Compares current config hash against cached config hash (different = rebuild)
5. Compares the Gro version, Node version, and installed-packages hash (different = rebuild)
6. If cache key matches, validates output files haven't been modified
7. If all checks pass, uses the cached build

Every cache key component is checked, and if any changes, the cache is invalidated.

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
