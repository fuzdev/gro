---
'@fuzdev/gro': minor
---

feat: read the Svelte config through Vite, lazily

Gro read `svelte.config.js` eagerly at module scope, so every invocation paid for it
twice - once on the main thread and once on the Node loader's worker thread - even for
tasks that never touch it. The main thread now loads it on demand and memoizes it, and
it's resolved from the project's Vite config rather than read from `svelte.config.js`
directly.

It's the same `vite.resolveConfig` call SvelteKit's own `load_config` makes, so Gro sees
exactly what SvelteKit sees: the inline options when a project passes them to
`sveltekit()`, and otherwise whatever SvelteKit loaded from `svelte.config.js` on its
own. That means projects keeping a `svelte.config.js` are unaffected - Vite reads it for
them - while projects configuring Svelte inline in `vite.config.ts` now work, along with
SvelteKit's own defaults instead of Gro's hand-rolled fallbacks. A Vite config that
fails to resolve now throws instead of being silently ignored.

Gro reads the project in the cwd and only that project. SvelteKit resolves its `files`
and `env.dir` against its own cwd rather than the Vite `root` it's handed, so a
directory parameter could only ever be half-honored, and there isn't one.

The Node loader resolves the config the same way as the main thread. It runs on a
worker thread, where `process.chdir` is unavailable, but Vite's config resolution
doesn't need it - only `@sveltejs/load-config` does, which is why Gro calls
`vite.resolveConfig` directly and drops that dependency.

The loader stays eager, though: resolving the config runs the `resolve` hook for each of
its own imports, so a hook that awaited the load it is part of would deadlock. That side
is now more expensive than reading `svelte.config.js` was, and it's on the critical path
of every invocation, so the win here is on the main thread rather than overall.

Resolving a Vite config is more than a read - it runs every plugin's `config` and
`configResolved` hooks, so it inherits their side effects. SvelteKit's rewrite of
`.svelte-kit/env.d.ts` is one, and Vite writing `process.env.NODE_ENV` when it's unset
is another. Gro restores `NODE_ENV` around the call, so the `development` Vite would
leave behind no longer reaches the `vite build` that `gro build` spawns.

Vite is now an optional peer dependency. A project with no Vite config gets the
conventional defaults rather than an error, so non-Vite projects keep working. Note that
a `svelte.config.js` is only read through Vite, so a project with one but no Vite config
gets those defaults too, not its own preprocessors, aliases, or compiler options - that
combination now warns, since it looks configured while being silently ignored.

A project that does have a Vite config but no Vite installed throws rather than falling
back, because falling back would mean compiling against the wrong config in silence.

The warning goes through the exported `svelte_config_log`, since this runs where there's
no logger to pass in - set `svelte_config_log.level = 'off'` to silence it.

Also fixes `parse_svelte_config` mutating the `compilerOptions` of the config passed to
it, and normalizes `kit.env.dir` to a project-relative path - it's serialized into the
generated `$env/dynamic/*` modules, so the absolute one Vite resolution produces would
bake the build machine's directory into server bundles.

Breaking changes:

- `TaskContext.svelte_config` and `GenContext.svelte_config` are now
  `Promise<ParsedSvelteConfig>` - `await` them
- `default_svelte_config` is replaced by `load_default_svelte_config()`, which takes no
  arguments
- `parse_svelte_config` takes `{svelte_config}` instead of `{dir_or_config}`, to parse an
  already-loaded config; omit it to resolve the project's
- `load_svelte_config` resolves the project's Vite config and takes no arguments;
  `GroConfig.svelte_config_filename` is removed, since Vite picks its own config file
- `has_sveltekit_app` is synchronous and takes a `PackageJson`, detecting
  `@sveltejs/kit` as a dependency rather than the presence of `svelte.config.js`
- `has_sveltekit_library` no longer takes a `ParsedSvelteConfig` - it reads the memoized
  one, and only after checking the `@sveltejs/package` dependency, so a project that
  isn't a library never reads the Svelte config
- `ROUTES_DIRNAME` is removed
- `SVELTE_CONFIG_FILENAME` and `VITE_CONFIG_FILENAME` are replaced by
  `SVELTE_CONFIG_FILENAMES` and `VITE_CONFIG_FILENAMES`, every filename SvelteKit and
  Vite accept for their configs. `gro format` now formats whichever of them a project
  has, not only `svelte.config.js` and `vite.config.ts`. `VITE_CONFIG_BASENAME` and
  `VITE_CONFIG_EXTENSIONS` are removed, subsumed by the filename list
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
