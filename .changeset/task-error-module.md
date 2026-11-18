---
'@fuzdev/gro': minor
---

feat: `TaskError`, `TaskErrorOptions`, `SilentError`, `is_task_error`, and `is_silent_error` move from `@fuzdev/gro/task.ts` to `@fuzdev/gro/task_error.ts`, which imports nothing that reaches the process or the filesystem, so code that throws or matches a task's failure loads without the task runner

**breaking** `@fuzdev/gro/task.ts` no longer exports them: import them from `@fuzdev/gro/task_error.ts`. `TaskError` stays on the bare `@fuzdev/gro` entry.
