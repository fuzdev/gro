---
'@fuzdev/gro': minor
---

feat: load the SvelteKit config lazily via
[`@sveltejs/load-config`](https://github.com/sveltejs/language-tools/tree/master/packages/load-config)

Gro read the SvelteKit config eagerly at module scope, so every invocation paid for
it twice - once on the main thread and once on the Node loader's worker thread - even
for tasks that never touch it. It's now loaded on demand and memoized, and reading it
goes through `@sveltejs/load-config`, which adds `svelte.config.{ts,mts,cjs,mjs}`
support, resolves through `vite.config` when one is present, and applies SvelteKit's
own defaults instead of Gro's hand-rolled fallbacks. A config that fails to load now
throws instead of being silently ignored.

The loader keeps reading the config at module scope and opts out of Vite resolution:
it runs on a worker thread, where Vite's resolution can't run because it calls
`process.chdir`. A custom `svelte_config_filename` also opts out, because Vite's
resolution finds `svelte.config.*` on its own.

Also fixes `parse_svelte_config` mutating the `compilerOptions` of the config passed
to it, and normalizes `kit.env.dir` to a project-relative path - it's serialized into
the generated `$env/dynamic/*` modules, so an absolute one would bake the build
machine's directory into server bundles.

Breaking changes:

- `TaskContext.svelte_config` and `GenContext.svelte_config` are now
  `Promise<ParsedSvelteConfig>` - `await` them
- `default_svelte_config` is replaced by `load_default_svelte_config()`
- `parse_svelte_config` takes `{dir, svelte_config}` instead of `{dir_or_config}`
- `ROUTES_DIRNAME` is removed
- `paths.lib`, `LIB_DIRNAME`, `LIB_PATH`, and `LIB_DIR` are the conventional `src/lib`
  rather than derived from `kit.files.lib`, so that `paths` stays free of the config.
  Read `lib_path` off `ParsedSvelteConfig` to honor a customized `files.lib`, and point
  `task_root_dirs` at it in `gro.config.ts` if tasks live there
- `MODULE_PATH_LIB_PREFIX` is always `$lib/`, matching SvelteKit, which names the alias
  `$lib` no matter where `files.lib` points - it was previously derived from `files.lib`
- `has_sveltekit_library` takes an optional `ParsedSvelteConfig` and checks the
  `@sveltejs/package` dependency before the lib directory
- the default config detects plugins inside `plugins()` instead of when the config
  loads, so tasks other than `dev` and `build` no longer trigger detection
