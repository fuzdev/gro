---
'@fuzdev/gro': minor
---

feat: a builtin's grants name the builtins it `invokes` as a record of `BuiltinInvokeGrant`s (`builtin_host.ts`), each `{}` or marked, so a runtime reads from the table which builtins a command's closure holds: `when` names a boolean arg of the invoking task that the invoke runs only under. `check`'s `build` is `{when: 'build'}`, so a plain `gro check`'s closure holds no `build`, and `run_task` refuses an invoke marked `when` an arg the invoking task's parsed args leave unset, as it refuses an invoke its grants don't name.

**breaking** `BuiltinTaskGrants`'s `invokes` is a record by builtin name, `invokes: {gen: {}}` for `invokes: ['gen']`.
