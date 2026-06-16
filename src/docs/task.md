# task

> task runner for
> [Gro](https://github.com/fuzdev/gro)

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
- tasks can be run from the CLI via a name (`gro foo`),
  which uses Gro's task resolution (see more below),
  or via paths that are absolute (`gro /path/to/foo`) or explicitly relative (`gro ./foo`)
- Gro automatically discovers all `*.task.ts|js` files
  in its configurable directory, so creating a new task
  is as simple as [creating a new file](#define-a-task), no config needed
  (defaults to `src/lib`, see the config option [`task_root_dirs`](./config.md#task_root_dirs))
- to view [the available tasks](https://github.com/fuzdev/gro/blob/main/src/docs/tasks.md)
  run `gro` with no arguments
- task definitions are just objects with an async `run` function and some optional properties,
  so composing tasks is explicit in your code, just like any other module
  (but there's also the helper `invoke_task`, see more below)
- the task object's `run` function has access to CLI args
- tasks optionally use [zod](https://github.com/colinhacks/zod) schemas
  for `args` types, runtime parsing with helpful validation errors,
  and generated help docs (`gro foo --help`), with DRY co-located definitions
- it's easy to call into or override any of Gro's builtin tasks,
  like [`gro test`](/src/lib/test.task.ts) and [`gro gen`](/src/lib/gen.task.ts) -
  your own versions with the same name take precedence, and you can invoke the base
  tasks using the `gro/` prefix, e.g. `gro gro/test`
  (tasks are also copy-paste friendly! just update the imports)
- it's fast because it imports only the modules imported by your invoked tasks, not every task's

The task runner's purpose is to provide an ergonomic interface
between the CLI, build tools, and app code.
As a developer, it's nice to be able to reuse TypeScript modules in every context.

## usage

### show all available tasks

```bash
# This looks through `src/lib` in both the current working directory and Gro's source
# for all files matching `*.task.ts|js` and logs them out with their args docs.
$ gro
```

The [config](./config.md) option [task_root_dirs](./config.md#task_root_dirs)
tells Gro where to search for tasks.

> Currently, only the first directory specified in `task_root_dirs` that's found on the filesystem
> will be used to automatically discover tasks, like when running `gro` without args.
> Please open an issue if you would like to see Gro be able to discover
> tasks in more than one directory - it will take some reworking of internals
> but it seems like the right design.

### show tasks in a directory

```bash
# Logs all `*.task.ts|js` files in `src/lib/some/dir` and `gro/src/lib/some/dir`.
# If no tasks are found, it displays an error.
$ gro some/dir
```

> To learn more about the Gro CLI path conventions,
> see [the `input_paths` comments](../lib/input_path.ts)

### run a task

```bash
# This runs `src/lib/some/file.task.ts`,
# or if it doesn't exist, `gro/src/lib/some/file.task.ts`.
# If neither exists, it displays an error.
$ gro some/file arg1 arg2 --arg3 example

# This runs `gro/src/lib/some/file.task.ts` directly
# without checking the current working directory,
# and displays an error if it doesn't exist.
$ gro gro/some/file

# Without colors:
$ NO_COLOR=1 gro
```

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
import {z} from 'zod';

export const Args = z.strictObject({
	arg: z.number().meta({description: 'example number arg'}).default(2),
});
export type Args = z.infer<typeof Args>;

export const task: Task = {
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
	TArgs = Args, // same as `z.infer<typeof Args>`
	TArgsSchema extends z.ZodType = z.ZodType,
	TReturn = unknown,
> {
	run: (ctx: TaskContext<TArgs>) => Promise<TReturn>;
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
	svelte_config: ParsedSvelteConfig;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}
```

### run a task inside another task with `invoke_task`

Because Gro tasks are just functions,
you can directly import them from within other tasks and run them.
However, we recommend using the `invoke_task` helper
for its ergonomics and automatic logging and diagnostics.

The `invoke_task` helper uses Gro's task resolution rules
to allow user code to override builtin tasks.
For example, Gro's `check.task.ts` calls `invoke_task('test')`
so that it calls your `src/lib/test.task.ts` if it exists
and falls back to `gro/src/lib/test.task.ts` if not.

It's less important to use `invoke_task` over explicit imports in user code
because you don't need to rely on the task override rules to get desired behavior,
but the logging and diagnostics it provides are nice to have.

```bash
gro some/file
```

```ts
// src/lib/some/file.task.ts
import type {Task} from '@fuzdev/gro';

export const task: Task = {
	run: async ({args, invoke_task}) => {
		// runs `src/lib/some/file.task.ts`, automatically forwarding `args`
		await invoke_task('some/file');
		// as documented above, the following is similar but lacks nice features:
		// await (await import('./some/file.task.js')).run(ctx);

		// runs `src/lib/other/file.task.ts` and falls back to `gro/src/other/file.task.ts`,
		// forwarding both custom args and a different event emitter (warning: spaghetti)
		await invoke_task(
			'other/file',
			{...args, optionally: 'extended'},
			optionalEventEmitterForSubtree,
			optionalDevFlagForSubtree,
			optionalFsForSubtree,
		);

		// runs `gro/src/lib/other/file.task.ts` directly, bypassing any local version
		await invoke_task('gro/other/file');
	},
};
```

### hook into one of [Gro's builtin tasks](./tasks.md)

```bash
# This normally loads Gro's version of the test task at `gro/src/lib/test.task.ts`,
# but projects can define `src/lib/test.task.ts` to extend or replace it.
$ gro test
```

```ts
// src/lib/test.task.ts
import type {Task} from '@fuzdev/gro';

export const task: Task = {
	run: async ({args, invoke_task}) => {
		await doSomethingFirst();
		// As discussed in the `invoke_task` section above,
		// it's possible to `import {task as groBuiltinTestTask} from '@fuzdev/gro/test.task.ts'`
		// and then call `groBuiltinTestTask.run` directly,
		// but that loses some important benefits.
		// Still, the task is available to import if you want it for any reason!
		await invoke_task('gro/test', {...args, optionally: 'extended'}, newEventEmitterForSubtree);
		await emailEveryoneWithTestResults();
	},
};
```

Note that when hooking into [Gro's builtin tasks](./tasks.md),
like `test.task.ts` above, you don't have to call its version.
You can copy/paste an existing task and customize it,
rewrite a task from scratch, compose them together, or whatever is needed for each project.

### task `Args`

The **`Args` property** of each `Task` (not the task context **`args` param** described above!)
is an optional [zod](https://github.com/colinhacks/zod) schema.
Using zod has some benefits:

- automatically print helpful text on the CLI with `gro` and `gro taskname --help`
- automated args parsing/validation using the schema with initialized defaults
- type safety using args in tasks
- concise source of truth

```ts
// src/lib/dosomething.task.ts
import type {Task} from '@fuzdev/gro';
import type {z} from 'zod';

export const Args = z.strictObject({
	_: z.array(z.string()).meta({description: 'rest args'}).default([]),
	yepyep: z.string().meta({description: 'helpful info'}).default('ya'),
	okcool: z.number().meta({description: 'that prints to the CLI'}).default(1234),
	maybee: z.boolean().meta({description: 'and optional args work too'}),
});

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

Running `gro dosomething --help` prints the schema information in a friendly format.

### task args forwarding

Some builtin Gro tasks call external commands like
[`svelte-kit`](https://github.com/sveltejs/kit),
[`vite`](https://github.com/vitejs/vite),
[`vitest`](https://github.com/vitest-dev/vitest),
[`tsc`](https://github.com/microsoft/typescript),
and [`prettier`](https://github.com/prettier/prettier).
Gro supports generic agnostic args forwarding to these tasks via the `--` pattern:
for example, to forward args to `svelte-kit` and `vitest`, no matter which task invokes them,
use `gro taskname --taskname-arg -- vitest --arg1 neat --arg2 22 -- svelte-kit --arg3`.

Any number of sections separated by `--` may be defined, and the first arg
that appears after each `--` is assumed to be the CLI command.
If `gro taskname` or its invoked tasks don't call `vitest` or `svelte-kit`,
the `--` args will be ignored.

There's one special case for task args forwarding: running Gro tasks.
If `gro` is the command following a `--`, e.g. the second `gro` of
`gro taskname -- gro taskname2 --a --b`,
then `--a` and `--b` will be forwarded to `taskname2`.
Forwarded args to Gro tasks override direct args, including args to `invoke_task`,
so `gro taskname --a 1 -- gro taskname --a 2` will invoke `taskname` with `{a: 2}`.

The `invoke_task` helper in the task context forwards the CLI args for the specified task.
CLI args take precedence over args passed directly to `invoke_task`.
This may not always be the desired behavior, but it gives the user more control,
because you can't change args in code you don't control.

### throwing errors

If a task encounters an error, normally it should throw rather than exiting the process.
This defers control to the caller, like your own parent tasks.

Often, errors that tasks encounter do not need a stack trace,
and we don't want the added noise to be logged.
To suppress logging the stack trace for an error,
throw a `TaskError`.

```ts
import {Task, TaskError} from '@fuzdev/gro';

export const task: Task = {
	run: async () => {
		if (someErrorCondition) {
			throw new TaskError('We hit a known error - ignore the stack trace!');
		}
	},
};
```

## why?

Gro usage on the command line (`gro <task_name_or_directory> [...flags]`)
looks a lot like using `node`.
What makes Gro different?

- The `*.task.ts` file name convention signals to Gro that your application
  contains task modules that conform to some interface.
  This allows them to be discoverable by convention,
  so running `gro` displays them all without any config, and it puts generic handles on them,
  enabling various verbs (e.g. `run`) and
  structured metadata (e.g. `summary` and args schemas for docs and validation).
- Tasks aren't just a script on the filesystem, they can be composed and inspected in code.
  Task modules do not have any side effects when imported,
  while Node scripts just execute when imported -
  their primary purpose is to cause side effects, and they're limited to the filesystem API.
  This is useful in many cases - for example `gro taskname --help`
  inspects the args schema and other metadata to print help to the console,
  and `gro` prints the `summary` property of each task it discovers.
  There's lots more to explore here, like task composition
  and improved DX with new capabilities.
- Tasks support CLI args that are validated and typesafe
  via colocated Zod schemas with minimal boilerplate.
- Tasks are forwarded CLI args when called via `invoke_task` in other tasks,
  so running `gro foo -- gro bar --a b` passes `{a: 'b'}` automatically to the `bar` task.
- Module resolution differs and leverages discoverability:
  - When a task name is given to Gro,
    it first searches `src/lib/` in the current working directory and
    falls back to searching the Gro directory.
    This allows your code and CLI commands to compose Gro's builtin tasks
    or override them without changing how you invoke them.
    Gro reserves no special behavior for its own commands -
    `gro test`, `gro gen`, and all the rest are just tasks that all follow the same rules.
    (see its task at [`src/lib/test.task.ts`](/src/lib/test.task.ts)).
  - When a directory is given to Gro,
    it prints all of the tasks found inside it,
    both relative to the current working directory and Gro's directory.
    So when you run `gro` by itself,
    it prints all tasks available both in your project and Gro.
  - The trailing `.task.ts` in the file path provided to `gro` is optional,
    so for example, `gro foo/bar` is the same as `gro foo/bar.task.ts`, a nice convenience.

## :turtle:<sub>:turtle:</sub><sub><sub>:turtle:</sub></sub>

Gro's task runner has many inspirations:

- [mgutz/task](https://github.com/mgutz/task)
- [Gulp](https://github.com/gulpjs/gulp)
- [@mgutz](https://github.com/mgutz)' [Projmate](https://github.com/projmate/projmate-core)
- [Grunt](https://github.com/gruntjs/grunt)
- [npm scripts](https://docs.npmjs.com/v8/using-npm/scripts)
