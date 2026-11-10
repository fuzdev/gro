---
'@fuzdev/gro': minor
---

feat: add `gro_plugin_server`, a dev server beside Vite restarted by the module graph

`gro_plugin_server({entry})` in `@fuzdev/gro/gro_plugin_server.ts` runs a server entry during
`gro dev` with Gro's loader, as `gro run` runs a file, and restarts it when the entry or anything it
imports changes, per the task context's `graph`, a burst of changes restarting it once. A server that
exits on its own is logged and comes back on the next change. Options are `entry`, `env_file`
(default `.env.development`, its vars beneath the host's environment, `null` for none), `env` (over
both, `NODE_ENV` defaulting to `development`), and `watch` (default `true`, `false` to start it
with no restarts). `gro dev --no-watch` starts no server, since it tears down right after setup.
`load_env_file` in `@fuzdev/gro/env.ts` loads one env file's vars, `null` for a missing file.

**breaking** `gro_plugin_deno_server` is removed. A Deno server's project moves to a Node entry under
`gro_plugin_server`: its `port` and `host` become `env: {PORT, HOST}` (or the env file's values),
`permissions` and `flags` have no counterpart, and `entry` has no default. `env` takes only strings,
an `undefined` value no longer unsetting a name.
