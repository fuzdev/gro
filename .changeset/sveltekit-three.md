---
"@fuzdev/gro": minor
---

**breaking** feat: support SvelteKit 3 only

- peer deps: `@sveltejs/kit` `^3` and `typescript` `^6`
- the Svelte config is read from the flat options SvelteKit 3's `sveltekit()` Vite plugin validates (no `kit` key, no `svelte.config.js`)
- `ParsedSvelteConfig` drops `alias`, `lib_path`, `private_prefix`, and `public_prefix`, and adds `src_path` and `version_name`; the lib directory is always `LIB_PATH` since SvelteKit removed `files.lib`
- SvelteKit's deprecated `alias` option is not supported (Gro warns when it's set, since Vite still applies it - replace package self-aliases like `'@fuzdev/foo': 'src/lib'` and `$routes` with `#lib/*` and `#routes/*`), and the loader and esbuild plugins no longer inject a `$lib` alias - use package.json subpath imports (`#lib/*`), which Node and esbuild resolve natively; the loader no longer resolves the Vite config at startup
- env shims follow SvelteKit 3's explicit env vars: `$app/env/public` and `$app/env/private` export the vars declared in `src/env.ts` (or `.js`, or `src/env/index.*`) via `defineEnvVars`, validated against their schemas; an `env.ts` that imports the `$app/env/*` it declares fails with an error instead of hanging
- the `$env/*` shims and the `PUBLIC_` prefix filtering are removed (SvelteKit 3 still serves `$env/*` deprecated, Gro doesn't)
- `load_env(dev, env_dir?, env_files?, ambient_env?)` drops its `visibility`, `public_prefix`, and `private_prefix` params and returns all vars unfiltered; the new `resolve_env_vars` picks out and validates the declared ones; the default env files are now Vite's (`.env`, `.env.local`, `.env.[mode]`, `.env.[mode].local`)
- `gro_plugin_server` bundles no longer inline static env vars: a bundled server reads them from its env files and ambient env at startup, and imports `@sveltejs/kit/env` (via the declarations) and `@fuzdev/gro/env.js` at runtime, so deploys need those installed
- the `$app/environment` shim is replaced by `$app/env`, whose `version` is now the config's `version.name` (SvelteKit 3 still serves `$app/environment` deprecated, Gro doesn't)
- the loader and filer resolve `#` subpath imports in the Vite convention too, `#lib/foo.js` naming `src/lib/foo.ts`
- the `$app/paths` shim matches SvelteKit 3 (`resolve`, `asset`, `match`) and now honors `paths.base` and `paths.assets` - its loader matchers had never matched
- `$app/navigation` and `$app/state` shims gain SvelteKit 3's `refreshAll`, `onNavigate`, `snapshot`, `pushState`, `replaceState`, and `page.shallow`
- the loader resolves its `$app` shims to its own modules rather than `@fuzdev/gro/*`, so they work under a global Gro
- `sveltekit_sync_if_obviously_needed` also syncs when `node_modules/$app/tsconfig.json` is missing
- `is_external_module` treats `#` subpath imports as internal instead of `$lib`
- renamed: `sveltekit_shim_env.ts` → `sveltekit_shim_app_env_vars.ts`, `sveltekit_shim_app_environment.ts` → `sveltekit_shim_app_env.ts`, `esbuild_plugin_sveltekit_shim_env.ts` → `esbuild_plugin_sveltekit_shim_app_env_vars.ts`, `render_sveltekit_shim_app_environment` → `render_sveltekit_shim_app_env`, `SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER` → `SVELTEKIT_SHIM_APP_ENV_MATCHER`
- removed: `svelte_config_cache.ts` (and the `.gro/svelte_config.json` cache), `esbuild_plugin_sveltekit_shim_alias`, `map_sveltekit_aliases`, `SVELTE_CONFIG_FILENAMES`, `SVELTEKIT_LIB_ALIAS`, `SVELTEKIT_ENV_MATCHER`, `warn_svelte_config_ignored`, `svelte_config_log`, `render_env_shim_module`, `merge_envs`, `is_public_env`, `is_private_env`, `esbuild_plugin_sveltekit_shim_env`
- added: `spawn_with_loader_args`, `resolve_env_vars`, `to_default_env_files`, `to_missing_module_id`
