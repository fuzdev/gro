---
'@fuzdev/gro': minor
---

move Gro's bare `process.*` onto the process host module, and keep the run's exit code on a broken pipe

Roots (tasks, `invoke.ts`, `gro.ts`, plugin modules, the rendered `$app/env/*` shims) import `process`
from the host module, and library code takes what it needs as parameters, with no `process.cwd()`
or `process.argv` defaults:

- `to_task_args`, `to_raw_rest_args`, `to_implicit_forwarded_argv`, `to_forwarded_argv`, `to_forwarded_args`,
  and `to_forwarded_args_by_command` take the CLI args after the CLI's own path (`process.args`) or
  the `raw_rest_args` derived from them, plus a `root` where a task path is normalized.
  `to_implicit_forwarded_argv`'s order is now `(task_name, cli_args, root, command?)`.
- `invoke_task` and `run_task` take a `raw_rest_args` option, and `invoke_cli` a fourth `raw_rest_args`.
- `to_task_path`, `to_input_path`, `to_input_paths`, and the `internal/task_hints` functions require a `root`.
- `load_from_env(fs_paths, ambient_env, key, paths?)` takes a plain record of the ambient env,
  and `LoadEnvOptions.ambient_env` is required.
- `configure_colored_output_with_path_replacement` and `map_child_process_output` take the output targets,
  `install_with_cache_healing` an `stderr` or an injected `run`, and writes are awaited.
- `spawn_with_loader` and `spawn_with_loader_args` take `node_env`.
- `run_svelte_package` takes `forwarded_argv`, `load_build_cache_dependencies` takes `runtime_version`,
  and the build cache's `node_version` becomes `runtime_version`, so an older `.gro/build.json` is a miss.

A broken stdout or stderr pipe no longer crashes the CLI or changes its exit code: the run finishes
with its own, so `gro foo | head` exits `0` only when `foo` passed, a failing task or a
`process.exitCode` keeps its code, and a run that never ends on its own, like `gro dev | head`,
runs until signalled. An uncaught error waits a few seconds at most for its output to be accepted
before exiting with its code.

`$app/env/*` read the ambient env through the process host module, so a task writing `process.env`
and then importing `$app/env/*` sees the value from when that module loaded, in the CLI before any task runs.
