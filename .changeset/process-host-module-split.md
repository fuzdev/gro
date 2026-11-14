---
'@fuzdev/gro': minor
---

feat: split the process host module into nine modules, one per kind of authority, each exporting a binding of its name: `env`, `args`, `exit` (`exit`, `set_exit_code`), `stdin`, `stdout`, `stderr`, `cwd`, `pid`, and `runtime_version`, with the errors in `env_error.ts` and `stdio_error.ts`

**breaking** `@fuzdev/gro/process.ts` and `@fuzdev/gro/process_error.ts` are removed. Import each member from its own module:

| was | now |
| --- | --- |
| `process.env` (`Env`) | `env` from `@fuzdev/gro/env.ts` (`Env` too) |
| `process.args` | `args` from `@fuzdev/gro/args.ts` |
| `process.exit(code?)` | `exit(code?)` from `@fuzdev/gro/exit.ts` |
| `process.set_exit_code(code)` | `set_exit_code(code)` from `@fuzdev/gro/exit.ts` |
| `process.stdin` (`ProcessInput`) | `stdin` from `@fuzdev/gro/stdin.ts` (`ProcessInput` too) |
| `process.stdout` (`ProcessOutput`) | `stdout` from `@fuzdev/gro/stdout.ts` (`ProcessOutput` too) |
| `process.stderr` | `stderr` from `@fuzdev/gro/stderr.ts` |
| `process.cwd()` | `cwd()` from `@fuzdev/gro/cwd.ts` |
| `process.pid` | `pid` from `@fuzdev/gro/pid.ts` |
| `process.runtime_version` | `runtime_version` from `@fuzdev/gro/runtime_version.ts` |
| `Process` | none: take the narrowest thing a function uses, a stream, a record, a string, or a function |
| `ProcessError` `not_granted`, from `env` | `EnvError` from `@fuzdev/gro/env_error.ts` |
| `ProcessError` `io_error`, from a stream | `StdioError` from `@fuzdev/gro/stdio_error.ts` |
| `is_process_error(error, kind?)` | `is_env_error(error, kind?)` or `is_stdio_error(error, kind?)` |
| `ProcessErrorKind` | `EnvErrorKind` or `StdioErrorKind` |

A module that imported `{process}` from Gro now reads Node's global under that name, so remove every `process.` use of the old import, not only the import line. `exit`, `set_exit_code`, and `cwd` are frozen functions that take no receiver, where the old methods threw on one other than `process`. Only `cwd.ts` checks the working directory when it loads, so importing another of these modules no longer throws in a removed working directory.

**breaking** `render_sveltekit_shim_app_env_vars` in `@fuzdev/gro/sveltekit_shim_app_env_vars.ts` takes `env_module_specifier` as the env host module's URL, where it took `process_module_specifier`; the env files module's URL, which `env_module_specifier` named before, is `env_files_module_specifier`.
