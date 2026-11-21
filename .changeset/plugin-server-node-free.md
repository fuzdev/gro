---
'@fuzdev/gro': minor
---

feat: `gro_plugin_server` imports no `node:` builtin when it loads, so a config that composes it loads where `node:` doesn't: it runs its entry with the host's `loader_path`, and `gro_helpers.ts` and `internal/server_restarts.ts` read paths as text

**breaking** `resolve_gro_module_path` is no longer exported, moved from `gro_helpers.ts` to the Node-only `internal/node_gro_module_path.ts`: a plugin that ran a file with Gro's loader takes the path from its context's `host.loader_path`. A hand-built `PluginHost` passes `loader_path`. `spawn_with_loader_args` and `spawn_with_loader` throw a `TypeError` for a relative loader path, which they read against the working directory before.
