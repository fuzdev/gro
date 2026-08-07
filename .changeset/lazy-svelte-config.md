---
'@fuzdev/gro': minor
---

feat: read the Svelte config through Vite, lazily

Gro read `svelte.config.js` eagerly at module scope, so every invocation paid for it
twice - once on the main thread and once on the Node loader's worker thread - even for
tasks that never touch it. It's now loaded on demand and memoized, and it's resolved
from the project's Vite config rather than read from `svelte.config.js` directly.

SvelteKit resolves its own config the same way, so Gro now sees exactly what `vite dev`
and `vite build` see: the inline options when a project passes them to `sveltekit()`,
and otherwise whatever SvelteKit loaded from `svelte.config.js` on its own. That means
projects keeping a `svelte.config.js` are unaffected - Vite reads it for them - while
projects configuring Svelte inline in `vite.config.ts` now work, along with SvelteKit's
own defaults instead of Gro's hand-rolled fallbacks. A Vite config that fails to
resolve now throws instead of being silently ignored.

The Node loader resolves the config the same way as the main thread. It runs on a
worker thread, where `process.chdir` is unavailable, but Vite's config resolution
doesn't need it - only `@sveltejs/load-config` does, which is why Gro calls
`vite.resolveConfig` directly and drops that dependency.

Vite is now an optional peer dependency. A project with no Vite config, or with no Vite
installed, gets the conventional defaults rather than an error, so non-Vite projects
keep working.

Also fixes `parse_svelte_config` mutating the `compilerOptions` of the config passed to
it, and normalizes `kit.env.dir` to a project-relative path - it's serialized into the
generated `$env/dynamic/*` modules, so the absolute one Vite resolution produces would
bake the build machine's directory into server bundles.

Breaking changes:

- `TaskContext.svelte_config` and `GenContext.svelte_config` are now
  `Promise<ParsedSvelteConfig>` - `await` them
- `default_svelte_config` is replaced by `load_default_svelte_config()`
- `parse_svelte_config` takes `{dir, svelte_config}` instead of `{dir_or_config}`, as a
  union that rejects contradictory options
- `load_svelte_config` resolves the Vite config at `dir` and takes only `{dir}`;
  `GroConfig.svelte_config_filename` is removed, since Vite picks its own config file
- `has_sveltekit_app` is synchronous and takes a `PackageJson`, detecting
  `@sveltejs/kit` as a dependency rather than the presence of `svelte.config.js`
- `has_sveltekit_library` takes an optional `ParsedSvelteConfig` and checks the
  `@sveltejs/package` dependency before the lib directory
- `ROUTES_DIRNAME` is removed
- `paths.lib`, `LIB_DIRNAME`, `LIB_PATH`, and `LIB_DIR` are the conventional `src/lib`
  rather than derived from `kit.files.lib`, so that `paths` stays free of the config.
  Read `lib_path` off `ParsedSvelteConfig` to honor a customized `files.lib`, and point
  `task_root_dirs` at it in `gro.config.ts` if tasks live there
- `MODULE_PATH_LIB_PREFIX` is always `$lib/`, matching SvelteKit, which names the alias
  `$lib` no matter where `files.lib` points - it was previously derived from `files.lib`
- the default config detects plugins inside `plugins()` instead of when the config
  loads, so tasks other than `dev` and `build` no longer trigger detection
- `CreateGroConfig` takes only `base_config` - its second `ParsedSvelteConfig` parameter
  was never passed, and passing one would defeat the lazy load
- `esbuild_plugin_svelte`'s default `svelte_compile_options` is
  `SVELTE_COMPILE_OPTIONS_DEFAULT` rather than the project's `compilerOptions`, because
  the default can't read the config synchronously - pass `svelte_compile_options` from a
  `ParsedSvelteConfig` to honor them
