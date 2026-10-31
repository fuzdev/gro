# task

> task runner for
> [Gro](https://github.com/fuzdev/gro)

> Where it's going: on the [runtime](runtime.md), `gro task <path>` and the builtin subcommands stay,
> and `gro check` infers each task's grants from its imports and diffs them against a committed policy file.

## contents

- [what](#what)
- [usage](#usage)
- [why?](#why)

## what

A Gro `Task` is just an object with a `run` function and some optional metadata.
Gro prefers conventions and code over configuration,
and its task runner leverages the filesystem as the API
and defers composition to the user in regular TypeScript modules.

> Tasks are a special Gro construct.
> If you want to simply execute regular TypeScript files,
> use the `gro run` task, which works like the normal `node` CLI
> but uses the Gro loader to support `.ts`.

- tasks are defined by naming files with the `.task.ts` and `.task.js` suffixes
- [Gro's builtin tasks](./tasks.md) run by name (`gro check`),
  and a name always means a builtin
- project tasks run by path (`gro task src/lib/foo.task.ts`),
  relative to the cwd (the project root) or absolute, `./` optional,
  with the `.task.ts` or `.task.js` suffix required;
  nothing is searched for, so creating a new task
  is as simple as [creating a new file](#define-a-task), no config needed
- `gro` with no arguments lists the builtins,
  and `gro task` lists the project's tasks under `src/`
- task definitions are just objects with an async `run` function and some optional properties,
  so composing tasks is explicit in your code, just like any other module
  (but there's also the helper `invoke_task`, see more below)
- the task object's `run` function has access to CLI args
- tasks optionally use [fuz_schema](https://schema.fuz.dev/) schemas
  for `args` types, runtime parsing with helpful validation errors,
  and generated help docs (`gro check --help`), with DRY co-located definitions
- a project task can wrap any of Gro's builtins,
  like [`gro test`](/src/lib/test.task.ts) and [`gro gen`](/src/lib/gen.task.ts),
  by invoking it by name (see [wrap a builtin](#wrap-one-of-gros-builtin-tasks));
  tasks are also copy-paste friendly, just update the imports
- it's fast because it imports only the modules imported by your invoked tasks, not every task's

The task runner's purpose is to provide an ergonomic interface
between the CLI, build tools, and app code.
As a developer, it's nice to be able to reuse TypeScript modules in every context.

## usage

### list tasks

```bash
# Lists Gro's builtins and the CLI usage, importing no task module.
$ gro

# Lists the project's `*.task.ts|js` files under `src/` with their summaries.
$ gro task

# Lists the tasks under a directory.
$ gro task src/lib/some/
```

The [config](./config.md) option [`search_filters`](./config.md#search_filters)
excludes paths from the listing.

### run a task

```bash
# Runs a builtin by name.
$ gro check

# Runs the project task at `src/lib/some/file.task.ts`.
$ gro task src/lib/some/file.task.ts arg1 arg2 --arg3 example

# Without colors:
$ NO_COLOR=1 gro check
```

A name that's no builtin is an error listing the builtins,
with the hint `gro task src/lib/foo.task.ts`, saying whether that file exists.
A path without `gro task`, like `gro src/lib/foo.task.ts`,
is an error naming the `gro task` form;
for a path without a task suffix, like `gro src/lib/foo`,
the error names what's there: `src/lib/foo.task.ts` or `.task.js` to run,
the directory to list, or nothing.
`gro task src/lib/foo` listing no tasks beside `src/lib/foo.task.ts` hints running it.
Flags go after the path: `gro task --a src/lib/foo.task.ts` is an error,
since `--a` would take the path as its value.

### define a task

```ts
// src/lib/some/file.task.ts
import type {Task} from '@fuzdev/gro';

export const task: Task = {
	run: async ({log, args}) => {
		log.info('CLI args', args); // => {_: ['arg1', 'arg2'], arg3: 'example'}
		await whatever();
	},
};
```

The minimum:

```ts
// src/lib/some/minimal.task.ts
export const task = {
	run: () => console.log('a minimal example'),
};
```

Minimum with [`Args`](#task-args):

```ts
// src/lib/some/withargs.task.ts
import type {Task} from '@fuzdev/gro';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import {meta} from '@fuzdev/fuz_schema/meta.ts';

export const Args = s.object({
	arg: s.with_default(meta(s.number, {description: 'example number arg'}), 2),
});
export type Args = s.Infer<typeof Args>;

export const task: Task<Args> = {
	Args,
	run: async ({args}) => {
		args.arg; // `number` that defaults to `2`
	},
};
```

### type `Task`

```ts
import type {Task} from '@fuzdev/gro';

export interface Task<
	TArgs = Args, // same as `s.Infer<typeof Args>`
	TArgsSchema extends Schema<Args, Args> = Schema<Args, Args>,
	TReturn = unknown,
> {
	run: (ctx: TaskContext<TArgs>) => TReturn | Promise<TReturn>;
	summary?: string;
	Args?: TArgsSchema;
}
```

### type `TaskContext`

```ts
import type {TaskContext} from '@fuzdev/gro';

export interface TaskContext<TArgs = object> {
	args: TArgs;
	config: GroConfig;
	svelte_config: Promise<ParsedSvelteConfig>;
	graph: ModuleGraph;
	watcher: Watcher;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}
```

`graph` is the project's module graph (the modules under `src/`, which imports which,
and their changes) and `watcher` a plain file watch over the same directory.
Both load on first use, so a task that never touches them never starts a watch;
`config.watch` sets what they watch.
A task invoked with `invoke_task` shares its invoker's `graph` and `watcher`.

### run a task inside another task with `invoke_task`

Because Gro tasks are just functions,
you can directly import them from within other tasks and run them.
However, we recommend using the `invoke_task` helper
for its ergonomics, automatic logging and diagnostics,
and [forwarded args](#task-args-forwarding).

`invoke_task` takes a builtin name or a task path by the same rule as the CLI:
an input containing a `/` or ending in `.task.ts` or `.task.js` is a path,
relative to the cwd (the project root, not the invoking task's directory) or absolute,
and anything else is a builtin name.
A name always reaches the builtin, so Gro's `check.task.ts` calling `invoke_task('test')`
runs the builtin `test` whatever tasks the project defines.

```ts
// src/lib/some/file.task.ts
import type {Task} from '@fuzdev/gro';

export const task: Task = {
	run: async ({args, invoke_task}) => {
		// runs the builtin `gen`
		await invoke_task('gen', {check: true});

		// runs `src/lib/other/file.task.ts`,
		// passing custom args and optionally a different config
		await invoke_task(
			'src/lib/other/file.task.ts',
			{...args, optionally: 'extended'},
			{config: optional_config},
		);

		// similar, but lacks the logging, diagnostics, and forwarded args:
		// await (await import('../other/file.task.ts')).task.run(ctx);
	},
};
```

### types for `invoke_task`

`invoke_task` checks task names and args at the type level.
A literal name must be a builtin,
and args are checked against the input type of the task's exported `Args` schema,
so args with defaults are optional and args without defaults are required.

Gro's builtins are typed out of the box.
To type your project's tasks, add a genfile that exports `gen_task_registry`:

```ts
// src/gro_tasks.gen.ts
import {gen_task_registry} from '@fuzdev/gro/task_registry.ts';

export const gen = gen_task_registry;
```

`gro gen` then writes `src/gro_tasks.ts`, which augments the `ProjectTasks` interface
with every task file under `src/`, keyed by its path relative to the project root
(`'src/lib/foo.task.ts'`), so `invoke_task('src/lib/foo.task.ts', ...)`
and `invoke_task('./src/lib/foo.task.ts', ...)` check against that task's args.
A literal task path missing from the registry accepts any args.
The file regenerates in `gro dev` when a task file is added or changed,
but `gro dev` ignores deletes, so run `gro gen` after removing a task
(`gro check` catches a stale file with `gro gen --check`).
Args are read from the task module's `Args` export, so export the schema:

```ts
export const Args = s.object({name: s.with_default(s.string, 'world')});
export type Args = s.Infer<typeof Args>;
```

A task module without an `Args` export accepts any args.

To invoke a task by a name that isn't a literal, like one computed at runtime,
widen it to `string`, which opts out of checking:

```ts
const task_name: string = to_task_name_somehow();
await invoke_task(task_name, {anything: 'goes'});
```

### wrap one of [Gro's builtin tasks](./tasks.md)

A project task never shadows a builtin: `gro test` always runs Gro's `test`.
To extend a builtin, write a project task that invokes it by name,
and point the project's `package.json` script at the task's path:

```ts
// src/lib/test.task.ts
import type {Task} from '@fuzdev/gro';
import {Args} from '@fuzdev/gro/test.task.ts';

// shares the builtin's `Args`, so `--help` and arg validation are unchanged
// and the args passed through below type-check against the builtin
export const task: Task<Args> = {
	Args,
	run: async ({args, invoke_task}) => {
		await do_something_first();
		await invoke_task('test', args); // always the builtin
		await email_everyone_with_test_results();
	},
};
```

```json
{
	"scripts": {
		"test": "gro task src/lib/test.task.ts"
	}
}
```

Builtins that invoke other builtins reach the builtins,
so `gro check` runs Gro's `test`, not the wrapper.
A wrapper doesn't have to call the builtin at all:
copy an existing task and customize it,
rewrite it from scratch, compose several, or whatever each project needs.

### task `Args`

The **`Args` property** of each `Task` (not the task context **`args` param** described above!)
is an optional [fuz_schema](https://schema.fuz.dev/) object schema,
one entry per arg, with `s.object` refusing unknown args.
Using a schema has some benefits:

- automatically print helpful text on the CLI with `--help`
- automated args parsing/validation using the schema with initialized defaults
- type safety using args in tasks
- concise source of truth

```ts
// src/lib/dosomething.task.ts
import type {Task} from '@fuzdev/gro';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import {meta} from '@fuzdev/fuz_schema/meta.ts';

export const Args = s.object({
	_: s.with_default_fn(meta(s.array(s.string), {description: 'rest args'}), () => []),
	yepyep: s.with_default(meta(s.string, {description: 'helpful info'}), 'ya'),
	okcool: s.with_default(meta(s.number, {description: 'that prints to the CLI'}), 1234),
	maybee: s.optional(meta(s.boolean, {description: 'and optional args work too'})),
});
export type Args = s.Infer<typeof Args>;

export const task: Task<Args> = {
	Args,
	run: async ({args}) => {
		args._; // Array<string>
		args.yepyep; // string
		args.okcool; // number
		args.maybee; // boolean | undefined
	},
};
```

Running `gro task src/lib/dosomething.task.ts --help` prints the schema information in a friendly format.

A `with_default` value is the same object on every parse,
so an array or object arg takes its default from `with_default_fn`,
which makes a new one each time, as `_` does above.

### task args forwarding

Some builtin Gro tasks call external commands like
[`eslint`](https://github.com/eslint/eslint),
[`vite`](https://github.com/vitejs/vite),
[`vitest`](https://github.com/vitest-dev/vitest),
and [`tsc`](https://github.com/microsoft/typescript).
Gro supports generic agnostic args forwarding to these tasks via the `--` pattern:
for example, to forward args to `eslint` and `vitest`, no matter which task invokes them,
use `gro check --check-arg -- vitest --arg1 neat --arg2 22 -- eslint --arg3`.

Any number of sections separated by `--` may be defined, and the first arg
that appears after each `--` is assumed to be the CLI command.
If `gro check` or its invoked tasks don't call `vitest` or `eslint`,
the `--` args will be ignored.
An outside command gets its section's text verbatim, never parsed and reserialized,
so `-- eslint --max-warnings -1 --no-cache` reaches `eslint` exactly as written.
When a task passes a flag of its own, like `lint`'s `--max-warnings 0`
or `typecheck`'s `--noEmit` to `tsc`, a forwarded flag of the same name
(`--name`, `--name=value`, or `--no-name`) replaces it.
The forwarded text follows Gro's own args, except that `--noEmit` follows it,
as `tsc --build` must come first.
A literal `--` can't be forwarded, because it separates the sections
(except to [`gro run`](#gro-run)'s script).

`gro test` also forwards to `vitest` the first section, unless it starts with `gro`
or the tool's own name (`vitest`), only up to the next `--`,
and only when the CLI invoked `gro test` directly:
`gro test -- --watch` passes `--watch` to `vitest`,
but `gro check -- --watch` passes nothing to the `test` that `check` invokes.

There's one special case for task args forwarding: running Gro tasks.
If `gro` is the command following a `--`, the args after the task are parsed and forwarded to it:
in `gro check -- gro test --a --b`, `--a` and `--b` go to the builtin `test`,
and in `gro check -- gro task src/lib/foo.task.ts --a`, `--a` goes to that project task,
its path normalized like `gro task` normalizes it (`./` optional).
A path without `task`, like `-- gro src/lib/foo.task.ts`, is an error, as it is on the CLI.
Forwarded args to Gro tasks override direct args, including args to `invoke_task`,
so `gro test --a 1 -- gro test --a 2` invokes `test` with `{a: 2}`.

The `invoke_task` helper in the task context forwards the CLI args for the specified task.
CLI args take precedence over args passed directly to `invoke_task`.
This may not always be the desired behavior, but it gives the user more control,
because you can't change args in code you don't control.

### gro run

`gro run <path> ...rest` runs a TypeScript or JS file with Gro's loader, like `node`.
Everything after the path goes to the script verbatim, including any `--`,
so `gro run foo.ts --help` passes `--help` to the script, not to Gro.
Before the path, `gro run` takes only `--help`, and any other flag is an error
saying to put the script's flags after the path.
Also like `node`, it prints no timings, and Gro exits with the script's exit code,
or for a script killed by a signal, `128` plus the signal's number, like a shell (`143` for `SIGTERM`).
`invoke_task('run', {_: [path, ...positionals], ...named})` serializes the named args
after the positionals, as code has no raw text to pass.

### throwing errors

If a task encounters an error, normally it should throw rather than exiting the process.
This defers control to the caller, like your own parent tasks.

Often, errors that tasks encounter do not need a stack trace,
and we don't want the added noise to be logged.
To suppress logging the stack trace for an error,
throw a `TaskError`.

```ts
import {type Task, TaskError} from '@fuzdev/gro';

export const task: Task = {
	run: async () => {
		if (someErrorCondition) {
			throw new TaskError('We hit a known error - ignore the stack trace!');
		}
	},
};
```

A failed task is reported once, where it fails:
`invoke_task` logs the error and throws a `SilentError` with the original error as its `cause`,
so an invoking task reports only that it failed.
The CLI exits with `1`, or with the `exit_code` of a `TaskError` thrown by the task it invoked,
an integer 1–255, like `throw new TaskError('failed', {exit_code: 3})`.

## why?

Gro usage on the command line (`gro <builtin> [...flags]`, `gro task <path> [...flags]`)
looks a lot like using `node`.
What makes Gro different?

- The `*.task.ts` file name convention signals to Gro that your application
  contains task modules that conform to some interface.
  This allows them to be discoverable by convention,
  so running `gro task` displays them all without any config, and it puts generic handles on them,
  enabling various verbs (e.g. `run`) and
  structured metadata (e.g. `summary` and args schemas for docs and validation).
- Tasks aren't just a script on the filesystem, they can be composed and inspected in code.
  Task modules do not have any side effects when imported,
  while Node scripts just execute when imported -
  their primary purpose is to cause side effects, and they're limited to the filesystem API.
  This is useful in many cases - for example `gro check --help`
  inspects the args schema and other metadata to print help to the console,
  and `gro task` prints the `summary` property of each task it lists.
  There's lots more to explore here, like task composition
  and improved DX with new capabilities.
- Tasks support CLI args that are validated and typesafe
  via colocated fuz_schema schemas with minimal boilerplate.
- Tasks are forwarded CLI args when called via `invoke_task` in other tasks,
  so running `gro check -- gro test --a b` passes `{a: 'b'}` automatically to the `test` task.
- Resolution is explicit, with no search:
  - A name means one of Gro's builtins and nothing else,
    so `gro test` and a builtin's `invoke_task('test')` mean the same task in every project.
  - A path means exactly one task file, relative to the project root or absolute,
    so `gro task src/lib/foo.task.ts` names the file it runs,
    and a project task is displayed and its forwarded args keyed by that path.

## :turtle:<sub>:turtle:</sub><sub><sub>:turtle:</sub></sub>

Gro's task runner has many inspirations:

- [mgutz/task](https://github.com/mgutz/task)
- [Gulp](https://github.com/gulpjs/gulp)
- [@mgutz](https://github.com/mgutz)' [Projmate](https://github.com/projmate/projmate-core)
- [Grunt](https://github.com/gruntjs/grunt)
- [npm scripts](https://docs.npmjs.com/v8/using-npm/scripts)
