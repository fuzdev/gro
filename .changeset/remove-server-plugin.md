---
'@fuzdev/gro': minor
---

**breaking** feat: remove `gro_plugin_server` and esbuild, which only it used; the default config no longer adds a Node server plugin for `src/lib/server/server.ts`, and esbuild is no longer a peer dependency. Removed with it:

- the rest of `gro_plugin_server.ts`: `has_server`, `SERVER_SOURCE_PATH`, `to_server_source_id`, `GroPluginServerOptions`, `Outpaths`, `CreateOutpaths`, and `to_default_outpaths`
- the esbuild plugins and helpers: `esbuild_plugin_svelte`, `esbuild_plugin_sveltekit_local_imports`, `esbuild_plugin_sveltekit_shim_app`, `esbuild_plugin_sveltekit_shim_app_env_vars`, and `esbuild_helpers.ts` (`print_build_result`, `to_define_import_meta_env`, `TS_TRANSFORM_OPTIONS_DEFAULT`)
- the task and plugin context's `watcher`, which only the server plugin used: `Watcher`, `create_node_watcher`, `NodeWatcherOptions`, and `RunTaskOptions.watcher`; `invoke_task`'s `watching` option becomes `graph`
- the `.gro/dev` output directory: `paths.build_dev`, `GRO_DEV_DIRNAME`, `GRO_DEV_DIR`, and `gro clean --build_dev`; `gro dev` no longer cleans it
- `clean_fs` takes independent `CleanFsOptions` (`gro_dir`, `dist_dirs`, `sveltekit`, `nodemodules`) instead of `build`, `build_dev`, and `build_dist`, where `build` also deleted the `dist_*` dirs, and drops its `rm_options` param; `discover_dist_dirs` in `build_cache.ts` finds the `dist_*` dirs for both
- `ParsedSvelteConfig.svelte_compile_module_options` keeps only `generate`, `rootDir`, and `warningFilter` of the compile options, since the loader sets `dev` and `filename` per module
- `SERVER_DIST_PATH`, so the default `search_filters` no longer skip `dist_server`
- `EVERYTHING_MATCHER`, `ResolvedSpecifier.namespace`, and the exports `PATH_SPECIFIER_MATCHER`, `strip_raw_query`, `SVELTE_COMPILE_OPTIONS_DEFAULT`, and `to_default_compile_module_options`
- the unused `tslib` and `esm-env` dependencies
- other unused API: `DevTaskContext`, `module.ts` (`is_external_module`, `MODULE_PATH_SRC_PREFIX`, `MODULE_PATH_LIB_PREFIX`), `Paths.config`, `base_path_to_path_id`, `plugin_replace` (filter the base plugins by name instead), and `ParsedSvelteConfig.assets_path`
- `SVELTEKIT_SHIM_APP_PATHS_MATCHER` and `SVELTEKIT_SHIM_APP_ENV_MATCHER`; `SVELTEKIT_SHIM_APP_SPECIFIERS` maps to shim module names instead of `@fuzdev/gro/*` specifiers; `ResolvedSpecifier.specifier`, which echoed the input; and the `has_vite_config` export
- `gro_plugin_deno_server`'s `watch` defaults to the task's `watch` instead of `dev`, so `gro dev --no-watch` no longer starts `deno run --watch`
- `GroConfig.js_cli` and `JS_CLI_DEFAULT`, which nothing read; `spawn_with_loader` drops its `js_cli` param and always spawns `node`
- `load_sveltekit_env_declarations` drops its `fresh` param; `resolve_sveltekit_env_entry` drops its `dir` param and resolves against the cwd; `RenderSveltekitShimAppEnvVarsOptions` drops `env_files` and `ambient_env`, takes one `entry: SveltekitEnvEntry | null` (`{id, variables}`, the path and its declarations) instead of `variables` and `entry_specifier`, and requires `env_module_specifier`
