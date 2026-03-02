# build

> these docs are for production builds, for development see [dev.md](dev.md)

## usage

The `gro build` task produces outputs for production:

```bash
gro build
```

This runs the configured Gro plugins, `setup -> adapt -> teardown`, in production mode.

If your project has a SvelteKit frontend,
[the default plugin](../lib/gro_plugin_sveltekit_app.ts) calls `vite build`,
forwarding any [`-- vite [...]` args](https://vitejs.dev/config/):

```bash
gro build -- vite --config my-config.js
```

### sync and install

By default, `gro build` runs `gro sync` before building
to generate code and update `package.json` exports,
and installs packages if needed:

```bash
gro build               # runs sync + install + build
gro build --no-sync     # skip sync step
gro build --no-install  # skip package install
```

Note: `--no-sync` is ignored if install runs (install requires sync to generate exports).

## build caching

Gro caches builds to skip expensive rebuilds when nothing has changed.

### cache key

The cache key has two components:

- **git commit hash** - tracks code, dependencies, and config files
- **`build_cache_config` hash** - optional, for tracking external inputs like environment variables
  or feature flags (see [cache configuration](#cache-configuration))

The cache invalidates when either component changes.

When the cache is valid:

```bash
gro build
# Build cache valid (from 2025-10-21T10:30:00.000Z) (use --force_build to rebuild)
# Skipping build, cache is valid
```

Force a rebuild:

```bash
gro build --force_build
```

## workspace requirements

The build cache requires a clean git workspace to function correctly.
This section covers workspace behavior, best practices, and CI/CD integration.

### dirty workspace behavior

The build cache only works with a clean git workspace.
With uncommitted changes, the git commit hash doesn't reflect your actual code state,
so Gro disables caching to ensure builds always match your working directory.

This conservative approach prevents a subtle issue:
uncommitted changes could be reverted after building,
leaving cached outputs from code that no longer exists in your workspace.

Behavior with uncommitted changes:

- cache checking is skipped - builds always run
- cache is not saved - no `.gro/build.json` written
- all build outputs are deleted before building - cache file,
  `build/`, `dist/`, and `dist_*/` removed to prevent stale state
- you'll see: `workspace has uncommitted changes - skipping build cache`

Outputs are deleted directly before running plugins.

### best practices

For reliable caching:

- **commit before building for production** - cache requires a clean workspace
- **use `build_cache_config` for external inputs** - environment variables,
  remote configs, or feature flags that affect the build but aren't in git
- **use `gro check --workspace` in CI** - enforces clean git state before building
- **avoid concurrent builds** - multiple simultaneous `gro build` processes can conflict

During development, uncommitted changes automatically disable caching,
so builds always reflect your working directory.

### post-build verification

After plugins finish building, Gro verifies the workspace didn't change during the build.
If tracked files were modified or untracked files were created, the build fails:

```
Error: Build process modified tracked files or created untracked files.

Git status after build:
  Modified: src/lib/foo.ts

Builds should only write to output directories (build/, dist/, etc.).
This usually indicates a plugin or build step is incorrectly modifying source files.
```

This ensures build reproducibility and prevents plugins from corrupting source code.

### CI/CD integration

> Don't commit `.gro/build.json` - keep it in `.gitignore`.

Caching `.gro/build.json` between CI runs is not beneficial - each commit has a unique cache key.

Basic CI workflow:

```bash
gro check --workspace  # ensure clean workspace
gro build              # build (uses cache if valid)
```

## troubleshooting

Cache not working as expected?

- **workspace must be clean** - check `git status`
- **enable debug logging** - `LOG_LEVEL=debug gro build`
- **corrupted cache** - delete `.gro/build.json` and rebuild
- **platform differences** - add platform/arch to `build_cache_config`
- **non-deterministic builds** - builds with timestamps/UUIDs/random data invalidate cache every run
  - diagnostic: if cache never hits, run `gro build` twice and compare outputs:
    ```bash
    gro build && cp -r build build_1
    gro build && diff -r build build_1
    ```
  - if files differ, your build is non-deterministic
  - common causes: `new Date()`, `Math.random()`, `process.pid` in build-time code
  - solution: use `import.meta.env.MODE` checks or build-time constants

> ⚠️ the cache is conservative - when in doubt, it rebuilds

For implementation details, see [cache implementation](#cache-implementation).

## advanced topics

For advanced use cases, you can customize cache behavior or extend it for plugins.

### cache configuration

If your build depends on external factors (environment variables, data files, feature flags),
use `build_cache_config` in `gro.config.ts` to invalidate the cache when they change:

```typescript
// gro.config.ts
export default {
	build_cache_config: {
		api_endpoint: process.env.PUBLIC_API_URL,
		data_version: fs.readFileSync('data/version.txt', 'utf-8'),
		features: {analytics: true, beta_ui: false},
	},
} satisfies GroConfig;
```

The config is hashed and never logged to protect sensitive values.
Any change triggers a rebuild.

See [config.md](config.md#build_cache_config) for more details on `build_cache_config`.

### extending the cache

To influence the cache, plugins should contribute configuration
in `gro.config.ts` before config normalization.
Since `build_cache_config` is hashed during config load, plugins running later cannot modify it.

Instead, include any plugin-specific cache factors in your `gro.config.ts`:

```ts
// gro.config.ts
export default {
	build_cache_config: {
		my_plugin_version: '1.0.0', // plugin version affects cache
		other_factor: process.env.MY_BUILD_FACTOR,
	},
};
```

See [`build.task.ts`](../lib/build.task.ts) for integration with the build pipeline.

## cache implementation

> implementation details for advanced debugging

The build cache ([`src/lib/build_cache.ts`](../lib/build_cache.ts)) validates builds are current
by tracking git commits and optional user config.

### how caching works

```mermaid
flowchart TD
    Start([gro build]) --> Force{--force_build?}
    Force -->|yes| Rebuild[skip cache, rebuild]
    Force -->|no| Dirty{workspace clean?}

    Dirty -->|no| Clean[Delete cache & outputs]
    Clean --> Rebuild

    Dirty -->|yes| Load{cache exists?}
    Load -->|no| Rebuild
    Load -->|yes| Keys{git commit +<br/>config match?}

    Keys -->|no| Rebuild
    Keys -->|yes| Valid{files valid?}

    Valid -->|no| Rebuild
    Valid -->|yes| Skip[use cached build]

    Rebuild --> Build[run build]
    Build --> Verify{workspace<br/>status changed?}

    Verify -->|yes| ThrowError[throw error]
    Verify -->|no| Save{workspace<br/>still clean?}

    Save -->|yes| Commit{git commit<br/>changed?}
    Save -->|no| SkipSave[skip cache save]

    Commit -->|yes| WarnSkip[warn, skip save]
    Commit -->|no| SaveCache[save cache]

    style Skip fill:#90EE90
    style Rebuild fill:#FFB6C1
```

### cache storage

Cache metadata is stored at `.gro/build.json`.
This location is independent of build outputs, so the cache survives manual deletion of `build/`.
Running `gro clean` deletes the cache along with the `.gro/` directory.

The `.gro/build.json` file stores:

```json
{
	"version": "1", // schema version
	"git_commit": "abc123...", // commit at build time
	"build_cache_config_hash": "...", // SHA-256 of user config
	"timestamp": "ISO-8601", // build completion time
	"outputs": [
		// all output files with hashes and stats
		{
			"path": "build/index.html",
			"hash": "...", // SHA-256 of file contents
			"size": 1024, // file size in bytes
			"mtime": 1729512000000, // modification time (ms since epoch)
			"ctime": 1729512000000, // change/creation time (ms since epoch)
			"mode": 33188 // Unix file permissions (0644)
		},
		{
			"path": "dist/index.js",
			"hash": "...",
			"size": 2048,
			"mtime": 1729512001000,
			"ctime": 1729512001000,
			"mode": 33188 // stored but not validated
		}
	]
}
```

### validation process

The cache validates builds through several checks:

1. load `.gro/build.json` - missing/corrupt = rebuild
2. check version = "1" - mismatch = rebuild
3. compare git commit - different = rebuild
4. compare config hash - different = rebuild
5. verify files exist - missing = rebuild
6. check file sizes - mismatch = rebuild (fast path optimization)
7. hash files in parallel - mismatch = rebuild
8. all pass = use cache

The hashing is usually fast but adds overhead proportional to output size.

**Output discovery:**

The cache automatically discovers all build output directories:

- `build/` - SvelteKit app output
- `dist/` - SvelteKit library output
- `dist_*` - plugin outputs (e.g., `dist_server`)

Within these directories:

- **regular files** - hashed with SHA-256 and validated on cache checks
- **symlinks** - skipped (not hashed or tracked)
- **directories** - recursively scanned for regular files

This ensures cache validation is comprehensive
while avoiding issues with symlink handling across platforms.

### security

The cache uses SHA-256 cryptographic hashing for:

- **output file integrity** - detects tampering or corruption
- **config protection** - `build_cache_config` is hashed, never logged raw (may contain secrets)
- **cache key stability** - deterministic hashes ensure consistent cache behavior

Hashes are computed via Node's `webcrypto.subtle.digest()`.

## plugins

`Plugin`s are objects that customize the behavior of `gro build` and `gro dev`.
They try to defer to underlying tools as much as possible, and exist to glue everything together.
For example, the library plugin internally uses
[`svelte-package`](https://svelte.dev/docs/kit/packaging).
See [gro_plugin_sveltekit_library.md](gro_plugin_sveltekit_library.md) to learn more.

## deploying and publishing

Now that we can produce builds, how do we share them with the world?

The [`gro deploy`](deploy.md) task outputs builds to a branch,
like for static publishing to GitHub pages.

The [`gro publish`](publish.md) task publishes packages to npm.

Both of these tasks call `gro build` internally,
and you can always run it manually if you're curious.
