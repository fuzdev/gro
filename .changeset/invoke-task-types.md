---
'@fuzdev/gro': minor
---

**breaking** feat: `invoke_task` checks task names and args at the type level - `InvokeTask` is generic over the name, a literal name must be a known task, and args are checked against the input type of the task's exported `Args` schema (required when it has required fields)

- builtins are typed by the generated `GroBuiltinTasks` (`gro_builtin_tasks.ts`), with `gro/`-prefixed names always reaching the builtin
- a project types its own tasks by exporting `gen_task_registry` as `gen` from a genfile like `src/gro_tasks.gen.ts`, which writes a module augmentation of `ProjectTasks`; a project task shadows the builtin with the same name, like runtime resolution
- a name widened to `string` (dynamic names, paths, directories) opts out with loose `Args` - a call with an unregistered literal, like a path-form `invoke_task('src/lib/foo')`, needs widening or the genfile
- an override that passes its untyped `args` through to a builtin, like `invoke_task('gro/check', args)`, can fail with no properties in common - type it with the builtin's schema, `Task<Args>` with `Args` from `@fuzdev/gro/check.task.ts`
- added: `task_registry.ts` with `ProjectTasks`, `TaskRegistry`, `TaskName`, `TaskArgsInput`, `task_registry_find`, the renderers, and `gen_task_registry`
