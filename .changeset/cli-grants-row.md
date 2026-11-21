---
'@fuzdev/gro': minor
---

feat: the CLI's own steps before any task have a grants row, `GRO_CLI_GRANTS`, which the summaries genfile declares and emits in `gro_builtin_tasks.ts` beside `GRO_BUILTIN_TASK_GRANTS`: reading the project and loading its config (`project_modules`, the config being the project's code). `gro_main` cuts the root's handles by that row with `create_builtin_host`, as `run_task` cuts a builtin's, and loads the config through it, so a runtime that holds a command to its closure's grants holds the config load to the row.

**breaking** `task_registry_render_builtins(entries, cli_grants, output_dir, origin_path)` takes the CLI's grants.
