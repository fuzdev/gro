---
'@fuzdev/gro': minor
---

**breaking** feat: `invoke_task` type-checks task names and args - a literal name must be a known task, and args are checked against the input type of the task module's exported `Args` schema, so defaulted args are optional and required args are required; Gro's builtins, including `gro/`-prefixed names, are typed out of the box

To upgrade:

- type your project's tasks with a genfile like `src/gro_tasks.gen.ts` that imports `gen_task_registry` from `@fuzdev/gro/task_registry.ts` and exports it with `export const gen = gen_task_registry;`, then run `gro gen` - it writes `src/gro_tasks.ts` augmenting `ProjectTasks`, and a project task shadows the builtin with the same name, like runtime resolution
- widen a name that isn't a known literal to `string`, which opts out of checking - dynamic names, `./` and absolute paths, directories, and alternate path forms like `src/lib/foo` for `foo`
- an override that passes its untyped `args` through to a builtin, like `invoke_task('gro/check', args)`, can fail with "no properties in common" - type it with the builtin's schema, `Task<Args>` with `Args` from `@fuzdev/gro/check.task.ts`

Added:

- `task_registry.ts` with `ProjectTasks`, `TaskRegistry`, `TaskName`, `TaskArgsInput`, `TaskRegistryEntry`, `task_registry_find`, `task_registry_render_project`, `task_registry_render_builtins`, and `gen_task_registry`
- `gro_builtin_tasks.ts` with the generated `GroBuiltinTasks`
- `InvokeTaskName`, `InvokeTaskArgs`, and `InvokeTaskRest` in `task.ts`
