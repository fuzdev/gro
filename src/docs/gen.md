# gen

> automated codegen by convention for
> [Gro](https://github.com/fuzdev/gro)

## motivation

The [`gro gen` task](/src/lib/gen.task.ts) helps us enhance our projects
with convention-based code/data/file generation (codegen) techniques.

Why? Codegen can produce cool results and unhalting pain.
Used well, codegen can improve performance, flexibility, consistency, and development speed.
As developers, automating our work is a natural thing to do,
and whether or not it's a wise thing to do,
`gro gen` can bring automation deeper into our code authoring workflows.

By convention, `gro gen` looks through `src/`
for any TypeScript files with `.gen.` in the file name,
and it outputs a file stripped of `.gen.` to the same directory.
The `*.gen.*` origin files export a `gen` function or config object with a `generate` function.
More flexibility is available when needed including multiple custom output files.

`gen` is implemented in Gro as [a task](/src/lib/gen.task.ts)
and [a plugin](/src/lib/gro_plugin_gen.ts).
By default it runs during development with `gro dev` and for production in `gro build`.

Normally you'll want to commit generated files to git,
but you can always gitignore a specific pattern like `*.ignore.*`
and name the output files accordingly.

Integrating codegen into our development process
is a simple idea with vast potential.
It lets us have a single source of truth for data
without compromising any of our code's runtime characteristics.
We can generate documentation, types,
`index.ts` files exporting directories,
data for a UI,
validators, tests, fakes,
and more by introspecting our data at buildtime,
which speeds up development
The goal is to leverage automation to increase the power we wield over our code
with a straightforward developer experience.
Ergonomics are key to unlocking codegen's full potential.

**Be aware** — this is a sharp tool! It should be used sparingly, only when it's a clear win.
It adds a layer of indirection between the code you write and run,
and it's possible to tie yourself into knots with dependencies.
Also, you could introduce security vulnerabilities
if you fail to escape certain inputs.

> ⚠️ Generated files should never be edited directly,
> because the next time `gro gen` or `gro sync` runs,
> any uncommitted changes will be lost!
> I considered making `gen` only write to files that have no uncommitted changes,
> but that would impede many workflows,
> and I don't want to nudge users towards a habit of always adding an override flag.
> I can see one possible improvement that lets the user
> opt into making gen write only to unchanged files for those workflows that don't mind it,
> so if you would like to see that or something similar please open an issue.

Inspirations include Lisp macros, the
[Svelte](https://github.com/sveltejs/svelte) compiler,
and [Zig](https://github.com/ziglang/zig)'s comptime.
(but `gro gen` is far more primitive)

## usage

The `gro gen` task looks for any files with `.gen.`
in the file name and tries to call an exported `gen`
function to generate one or more output files,
and then it writes the results to the filesystem.

```bash
gro gen # runs codegen for all *.gen.* files in src/
gro gen --check # exits with error code 1 if anything is new or different; no-op to the fs
```

### generate arbitrary TypeScript

Given `src/script.gen.ts`:

```ts
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = () => {
	const message = 'generated';
	return `console.log('${message} a ${typeof message}')`;
};
```

Outputs `src/script.ts`:

```ts
console.log('generated a string');
```

### gen types

Gen files export either a function or object with a `generate` function:

```ts
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = (ctx) => {
	return 'generated content';
};

export const gen: Gen = {
	generate: (ctx) => 'generated content',
	dependencies, // optional, see docs below
};
```

The generate function receives a `GenContext` object:

```ts
export interface GenContext {
	config: GroConfig;
	svelte_config: ParsedSvelteConfig;
	filer: Filer;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
	/**
	 * Same as `import.meta.url` but in path form.
	 */
	origin_id: PathId;
	/**
	 * The `origin_id` relative to the root dir.
	 */
	origin_path: string;
	/**
	 * The file that triggered dependency checking.
	 * Only available when resolving dependencies dynamically.
	 * `undefined` during actual generation.
	 */
	changed_file_id: PathId | undefined;
}
// export const gen: Gen = ({config, svelte_config, origin_id, origin_path, log}) => {
```

### generate other filetypes

Files with any extension can be generated without configuration.
If the origin file name ends with the pattern `.gen.*.ts`,
the default output file name is stripped of its trailing `.ts`.

Given `src/markup.gen.html.ts`:

```ts
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = () => {
	const body = 'hi';
	return `
		<!DOCTYPE html>
		<html>
			<body>
				${body}
			</body>
		</html>
	`;
};
```

Outputs `src/markup.html`:

```html
<!doctype html>
<html>
	<body>
		hi
	</body>
</html>
```

### generate a custom file name or write to a different directory

The `gen` function can return an object with custom configuration.

Given `src/somewhere/originalName.gen.ts`:

```ts
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = () => {
	const message = 'output path can be relative and name can be anything';
	return {
		content: `console.log('${message}')`,
		filename: '../elsewhere/otherName.ts',
		format: optional_boolean_that_defaults_to_true,
	};
};
```

Outputs `src/elsewhere/otherName.ts`:

```ts
console.log('output path can be relative and name can be anything');
```

### generate multiple custom files

The `gen` function can also return an array of files.

Given `src/thing.gen.ts`:

```ts
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = () => {
	const fieldValue = 1;
	return [
		{
			content: `
				import {Thing} from './index';
				export const isThing = (t: any): t is Thing => t?.field === ${fieldValue};
			`,
		},
		{
			content: `export interface Thing { field: ${typeof fieldValue} }`,
			filename: 'types.ts',
		},
		{
			content: `{"field": ${fieldValue}}`,
			filename: 'data/thing.json',
		},
	];
};
```

Outputs `src/thing.ts`:

```ts
import type {Thing} from './index.ts';
export const isThing = (t: any): t is Thing => t?.field === 1;
```

and `src/types.ts`:

```ts
export interface Thing {
	field: number;
}
```

and `src/data/thing.json`:

```json
{
	"field": 1
}
```

### dependencies

Gen files can declare dependencies to control when they regenerate in watch mode.
By default, they regenerate only when their imported dependencies or the file itself change.
The `dependencies` option provides fine-grained control:

Regenerate on all file changes:

```ts
import type {GenConfig} from '@fuzdev/gro';

export const gen: GenConfig = {
	generate: () => 'returns generated contents',
	dependencies: 'all',
};
```

Static configuration with patterns and files:

```ts
export const gen: GenConfig = {
	generate: () => 'returns generated contents',
	dependencies: {
		patterns: [/\.json$/, /config\//], // regex patterns to match file paths
		files: ['src/data/schema.ts', 'package.json'], // specific file paths
	},
};
```

Dynamic resolver function:

```ts
export const gen: GenConfig = {
	generate: () => 'returns generated contents',
	dependencies: (ctx) => {
		return ctx.changed_file_id?.endsWith('.json')
			? {files: ['package.json', ctx.changed_file_id]}
			: null; // same as `{}` and `{patterns: [], files: []}`
	},
};
```

### checking generated files

Generating code directly in your git-committed source is powerful and transparent,
but a downside is that generated artifacts can become stale
outside of normal `gro dev` and `gro build` workflows.
It's often helpful to check if any generated files are new or different than what's on disk,
like in CI or before publishing a build.
The `check` CLI argument can be passed to perform this check
instead of writing the generated files to disk.

```bash
gro gen --check # exits with error code 1 if anything is new or different; no-op to the fs
```

or in code:

```ts
import type {Task} from '@fuzdev/gro';

export const task: Task = {
	run: async ({args, invoke_task}) => {
		// this throws a `TaskError` if anything is new or different
		await invoke_task('gen', {...args, check: true});
	},
};
```

Gro uses this in [`check.task.ts`](/src/lib/check.task.ts)
which is called during `gro publish`, and it's recommended in CI.
(see [Gro's example `check.yml`](/.github/workflows/check.yml))
