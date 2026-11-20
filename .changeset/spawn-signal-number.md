---
'@fuzdev/gro': minor
---

feat: a `SpawnResult` carries `signal_number` beside `signal`, the number of the signal that ended the child, so a shell's exit code for it is `128 + signal_number` with no `node:os` import, the same on the runtime's builtin; on a `timed_out` or `aborted` result it's `null` exactly when `signal` is

**breaking** `to_signal_exit_code` in `@fuzdev/gro/gro_helpers.ts` is removed: use `128 + result.signal_number`. Code that builds a `signaled`, `timed_out`, or `aborted` result, like a test double, adds the field.
