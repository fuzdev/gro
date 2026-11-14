---
'@fuzdev/gro': minor
---

feat: rename `TaskContext.args` to `task_args`, so a task's parsed args never shadow an imported `args`

**breaking** every task's `run` reads `task_args` where it read `args`, like `run: async ({task_args, log}) => {...}`, and so does any `PluginContext` consumer and any code that builds a `TaskContext`. The task's `Args` schema export and `Task<TArgs>` are unchanged.
