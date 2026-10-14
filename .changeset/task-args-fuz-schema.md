---
'@fuzdev/gro': minor
---

**breaking** feat: task `Args` are `@fuzdev/fuz_schema` schemas instead of zod - Gro parses task args with `args_parse` from `@fuzdev/fuz_util` `0.73.0`, which takes fuz_schema schemas, and builds `--help` with fuz_schema's `introspect.ts`

To upgrade:

- install `@fuzdev/fuz_schema` `^0.2.0`, a new peer dep, and `@fuzdev/fuz_util` `>=0.73.0`, the new peer floor
- write each task's `Args` with fuz_schema, importing `* as s` from `@fuzdev/fuz_schema/schema.ts` and `meta` from `@fuzdev/fuz_schema/meta.ts`: `z.strictObject({...})` becomes `s.object({...})`, `z.boolean().meta({description}).default(false)` becomes `s.with_default(meta(s.boolean, {description}), false)`, `.optional()` becomes `s.optional(...)`, `z.enum([...])` becomes `s.one_of([...])`, and `z.infer` becomes `s.Infer`
- give an array or object arg its default with `s.with_default_fn(schema, () => [])`, because a `with_default` value is the same object on every parse, where zod copied it
- write zod's `.default(x).optional()` as `s.with_default(schema, x)`, because an `s.optional` outside a default leaves a missing key out and the default never applies, where zod applied it
- call `s.parse_or_throw(Args, value)` where a test called a zod method like `Args.parse(value)`

Changed:

- `--help` quotes string literals as JSON, so `"public"` where it printed `'public'`
- `--help` takes each arg's type from fuz_schema's `format_schema_type` for every kind, so a union or array arg shows its real type where it showed `string | Array<string>` or `Array<string>`
- a `pipe` stops at its first failing step, so an arg that fails an earlier step no longer also reports a later one, like a `max_length`
- a task args validation error lists each issue as fuz_schema's `format_issue` text, like `sync: expected boolean, received string`, instead of zod's `prettifyError` text
- `gro run` checks its named args once its `_` passes, and its error names the first bad arg only
- `ChangesetAccess`, `ChangesetBump`, `InputPath`, and `RawInputPath` are fuz_schema schemas, and `Task`'s `TArgsSchema` extends `Schema<Args, Args>` from fuz_schema
- `invoke_task` types a task's args as its fuz_schema input, and as `Record<string, never>` for an `Args` with no keys, so an arg-less task still refuses every arg
