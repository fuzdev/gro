---
'@fuzdev/gro': minor
---

feat: a builtin's grants name the builtins it `invokes` as a record of `BuiltinInvokeGrant`s (`builtin_host.ts`), each `{}` or marked, so a runtime reads from the table which builtins a command's closure holds: `when` names a boolean arg of the invoking task that the invoke runs only while it's on, its default and its `no-` dual applied. `check`'s `build` is `{when: 'build'}` and `sync` `{when: 'install'}`, so a plain `gro check`'s closure holds neither, and each step `check` can skip is marked by its own flag (`test: {when: 'test'}`), so `gro check --no-test`'s closure holds no `test`. `run_task` refuses an invoke marked `when` an arg the invoking task's parsed args leave off, as it refuses an invoke its grants don't name. `BuiltinTaskEntry` has `Args`, the schema its args are parsed by.

**breaking** `BuiltinTaskGrants`'s `invokes` is a record by builtin name, `invokes: {gen: {}}` for `invokes: ['gen']`.
