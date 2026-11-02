---
'@fuzdev/gro': minor
---

feat: a `TaskError` or `SilentError` subclass is recognized as one, so a task throwing a `TaskError` subclass reports its message instead of an unexpected error with a stack; add `is_task_error` and `is_silent_error` to `task.ts`, which recognize the classes and their subclasses from any copy of Gro
