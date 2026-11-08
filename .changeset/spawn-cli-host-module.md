---
'@fuzdev/gro': minor
---

move `cli.ts` onto the spawn host module

`find_cli` is `find_cli(spawn, name, options?)`, synchronous, returning a `SpawnProgram` or `null`,
and `find_local_cli` finds only the project's `node_modules/.bin`. `Cli`, `resolve_cli`, and `to_cli_name`
are removed. `spawn_cli` and `spawn_cli_process` take a `SpawnProgram` and return a `SpawnResult` and a
`SpawnChild`, with the new `spawn_result_is_ok` and `to_spawn_result_message` to check a result.
`sveltekit_sync`, `sveltekit_sync_if_available`, `sveltekit_sync_if_obviously_needed`, and
`run_svelte_package` take a `Spawn`, and their CLI options take a name only.
`map_child_process_output` and `configure_colored_output_with_path_replacement` take a `SpawnChild`
and return a promise that resolves once its output is written.

A CLI found through `find_cli` or `find_local_cli` now fails for a file the kernel won't run, without
execute permission or neither `#!` nor ELF, rather than being found by existence alone. A `--*_cli`
naming an absolute path or one above the project is refused. These and a program that can't start
are a `TaskError` with the message of the `SpawnError`, which is its `cause`, so a task fails with
that message instead of an unexpected error. A CLI that isn't found is still `null` from `find_cli`.
A program found on `PATH` gets its bare name as `argv[0]` rather than its full path, and a task's
child gets the environment as of the handle's creation, not of the spawn.
