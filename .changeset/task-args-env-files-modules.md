---
'@fuzdev/gro': minor
---

feat: rename the `args.ts` module to `task_args.ts` and the `env.ts` module to `env_files.ts`, freeing the names for host modules

**breaking** import `@fuzdev/gro/task_args.ts` where you imported `@fuzdev/gro/args.ts` (`CliTaskArgs`, `to_task_args`, `to_raw_rest_args`, the forwarded-argv functions), and `@fuzdev/gro/env_files.ts` where you imported `@fuzdev/gro/env.ts` (`load_env`, `load_from_env`, `load_env_file`, `to_default_env_files`, `resolve_env_vars`). The exports keep their names.

**breaking** `render_sveltekit_shim_app_env_vars` in `@fuzdev/gro/sveltekit_shim_app_env_vars.ts` takes `env_files_module_specifier` where it took `env_module_specifier`.
