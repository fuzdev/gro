---
'@fuzdev/gro': minor
---

feat: a plugin takes its handles from its context's `host`, a `PluginHost` (`plugin.ts`: `fs_paths`, `env`, `spawn`, `import_module`) that `gro dev` and `gro build` open for their plugins, so the default config and Gro's plugins import no host module when they load, and loading the config, which every command does, reaches none through them. `gro_plugin_sveltekit_app` and `gro_plugin_sveltekit_library` read the forwarded args from the context's `raw_rest_args` rather than the `args` host module.

**breaking** `to_plugin_context(ctx, host, dev, watch)` takes the `PluginHost`, and a hand-built `PluginContext` passes `host`.
