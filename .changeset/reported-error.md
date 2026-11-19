---
'@fuzdev/gro': minor
---

**breaking** feat: rename `SilentError` to `ReportedError` and `is_silent_error` to `is_reported_error`, in `@fuzdev/gro/task_error.ts`: the name states what its thrower promises, that the failure is already reported, where "silent" named only what the CLI does with it. Rename the imports and uses; nothing else changes. A copy of Gro before this release throws a `SilentError` that `is_reported_error` doesn't match.
