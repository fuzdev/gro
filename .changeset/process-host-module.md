---
'@fuzdev/gro': minor
---

add the process host module, `process.ts` and `process_error.ts`

`process` is a frozen handle on the running process: `env` (`get`, `get_all`, `narrow`, a
read-only snapshot taken at load, narrowing by exact names only), `args`, `cwd()`, `exit`,
`set_exit_code`, `stdin`, `stdout`, `stderr`, `pid`, and `runtime_version`. `ProcessError` has
kinds `not_granted`, a name outside a narrowed `Env`, and `io_error`, with `is_process_error`.
