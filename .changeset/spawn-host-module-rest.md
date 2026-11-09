---
'@fuzdev/gro': minor
---

move every remaining spawn onto the spawn host module, off `@fuzdev/fuz_util/process.ts`

`install_with_cache_healing` and `install_with_cache_healing_or_throw` take a `Spawn` first, and
their `cwd` is an fs handle; `NpmCommandRunner` receives the pinned `SpawnProgram`. Its environment
is every host variable but `NODE_ENV`, `npm_config_production`, and `npm_config_omit`, unset for
the child as before. `to_install_program_options` returns the program's options, and
`sanitize_install_env` and the helpers' `env` option are removed, no caller having used them.

`SpawnProgramOptions` gains `omit_env`, exact case-sensitive names `pass_env: 'all'` leaves out,
refused with any other `pass_env`.

`spawn_with_loader` takes the `node` program first, the root deciding its environment, and returns
the spawn host module's `SpawnResult`.

New `spawn_restartable.ts`, `spawn_restartable(program, args, options)`, a restartable child over a
`SpawnProgram` for what `spawn_restartable_process` did. `gro_plugin_deno_server` uses it, resolving
`deno` once at setup with the host's environment plus its explicit values; a missing `deno` now
fails `gro dev` with the host module's message instead of logging a started server that never ran.

`chmod` for a package's bins and `<pm_cli> link -f` go through the host module, a missing `chmod` or
package manager reported as the `SpawnError` message. `gro deploy` writes the initial `.gitkeep`
through `fs`. An uncaught `TaskError` prints its message without a stack. `pm_cli` is a name on `PATH` or a
project-relative path, and an absolute path is refused.

A project still using `fuz_util`'s spawn for a long-lived child (a `gro.config` plugin) no longer has
it killed by Gro on an uncaught exception. `spawn_restartable` is the replacement.
