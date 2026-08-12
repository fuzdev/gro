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

The Node loader and the `Filer` need only the `alias` map, so the new
`svelte_config_cache.ts` caches that much at `.gro/svelte_config.json`, keyed on the mtime
and size of every Vite and Svelte config filename plus `package.json` - absent ones
included, so that adding a config invalidates too. The blind spot is a Vite config that
imports the module declaring `kit.alias` or `kit.files.lib`: editing that module doesn't
invalidate, and the stale alias surfaces as an unresolved import rather than a bad
compile. `gro clean` clears it, along with the rest of `.gro`.

Measured in this repo, `gro --version` goes from ~1.24s to ~0.98s. A resolution costs
~700-900ms on its own, but the loader's thread overlaps with main-thread startup, so an
invocation only sheds the part that didn't overlap.

Resolving a Vite config is more than a read - it runs every plugin's `config` and
`configResolved` hooks, so it inherits their side effects. SvelteKit's rewrite of
`.svelte-kit/env.d.ts` is one, and Vite writing `process.env.NODE_ENV` when it's unset
is another. Gro restores `NODE_ENV` around the call, so the `development` Vite would
leave behind no longer reaches the `vite build` that `gro build` spawns.

The `compilerOptions` read back for a plain Svelte project are `vite-plugin-svelte`'s
*resolved* options rather than the user's, so they arrive with that plugin's own `css`,
`dev`, and `hmr` mixed in, resolved for the `build`/`production` pass. It deletes
`generate`, so Gro's server default survives, and the loader always sets `dev` itself.
SvelteKit projects are unaffected - their `api.options` is the Svelte config as authored.

Vite is now an optional peer dependency. A project with no Vite config gets the
conventional defaults rather than an error, so non-Vite projects keep working. Note that
a `svelte.config.js` is only read through Vite, so a project with one but no Vite config
gets those defaults too, not its own preprocessors, aliases, or compiler options - as does
one whose Vite config configures no Svelte plugin, the likelier case: a `vite.config.ts`
that only sets up Vitest, alongside a `svelte.config.js` doing the real configuring, lands
exactly there. Unlike SvelteKit, Gro doesn't fall back to importing the Svelte config
itself.

Both cases now warn, since that Svelte config looks like it's configuring the project
while being ignored - a project with no Svelte config stays quiet, having nothing to
ignore. The warnings go through the exported `svelte_config_log`, since this runs where
there's no logger to pass in; set `svelte_config_log.level = 'off'` to silence them.
Having a Vite config but no Vite installed throws rather than falling back, because
falling back would mean compiling against the wrong config in silence.

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
  isn't a library never reads the Svelte config. Both it and `has_sveltekit_app` also
  drop their unused `dep_name` parameter and use the dependency-name constants directly
- both detect through `dependencies` and `devDependencies` only. A peer dep declares what
  a package works alongside, not what it is, so counting it would run `vite build` over a
  library that peers on SvelteKit and has no app. `package_json_has_dependency` takes a
  third `include_peer` param for this, defaulting to `true` as before
- `GenContext` is built by the new `create_gen_context`, shared by generation and
  dependency resolution so they can't drift
- `ROUTES_DIRNAME` is removed
- `SVELTE_CONFIG_FILENAME` and `VITE_CONFIG_FILENAME` are replaced by
  `SVELTE_CONFIG_FILENAMES` and `VITE_CONFIG_FILENAMES`, every filename Vite and
  `vite-plugin-svelte` accept for their configs - the Svelte side is that plugin's list
  rather than SvelteKit's narrower fallback one, so a `svelte.config.mjs` still keys the
  cache and gets formatted. `gro format` now formats whichever of them a project has,
  not only `svelte.config.js` and `vite.config.ts`
- `paths.lib`, `LIB_DIRNAME`, `LIB_PATH`, and `LIB_DIR` are the conventional `src/lib`
  rather than derived from `kit.files.lib`, so that `paths` stays free of the config, and
  the three constants move from `paths.ts` to `constants.ts` - they only lived in
  `paths.ts` because they used to read the config. Everything that has to honor a
  customized `files.lib` reads `lib_path` off `ParsedSvelteConfig` instead - the
  `package.json` exports automation, the server plugin's entry point and `outbase`, and
  `has_sveltekit_library`. Point `task_root_dirs` at it in `gro.config.ts` if tasks live
  there
- `package_json_sync`'s `exports_dir` param no longer defaults to `paths.lib`; when
  omitted it's the Svelte config's `lib_path`, which `has_sveltekit_library` has already
  resolved by then. Searching the conventional `src/lib` for a project that moved its lib
  directory found nothing and replaced the whole `exports` map with a single entry
- `gro_plugin_server`'s `SERVER_SOURCE_ID` is replaced by `SERVER_SOURCE_PATH` (relative
  to the lib directory) and `to_server_source_id(lib_path)`. Its `entry_points` and
  `outpaths` defaults now resolve in `setup` rather than at plugin creation, since
  reading `lib_path` is async; the outpaths default is exported as `to_default_outpaths`.
  `has_server()` takes an optional path and defaults to the configured lib directory
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
- `dev` and `build` widen their `TaskContext` with the new `to_plugin_context` instead of
  spreading it, because spreading calls the lazy `svelte_config` getter and resolves the
  config for plugin sets that never read it
