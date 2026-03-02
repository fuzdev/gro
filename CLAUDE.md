# Gro architecture reference

> Task runner and toolkit extending SvelteKit - conventions over configuration,
> filesystem as API

Gro (`@fuzdev/gro`) is a dev tool for building TypeScript projects with
SvelteKit, providing: convention-based task runner, Node loader for
TypeScript/Svelte/SvelteKit modules, code generation system, plugin
architecture, and integrations with Vite, esbuild, Vitest, Prettier, ESLint, and
Changesets. It's designed around conventions and the filesystem, not
configuration files.

## Scope

Gro is a **devDependency** - it runs at build time, not in production bundles.

Key responsibilities:

- Task runner and CLI (`gro dev`, `gro build`, `gro test`, etc.)
- Code generation system (`.gen.ts` files)
- Node loader for TypeScript/Svelte without compilation step
- Plugin system for dev/build workflows
- `src_json` generation - analyzes TypeScript/Svelte source to produce metadata
  consumed by fuz_ui's API documentation system
- `.well-known/package.json` and `.well-known/src.json` publishing for package
  metadata

## Core systems

### Task runner

[Full documentation: src/docs/task.md](src/docs/task.md)

Tasks are TypeScript modules with `.task.ts` suffix that export a `task` object
with a `run` function. The task runner discovers tasks by convention, resolves
them through a search path, and provides a context with args, config, logger,
timings, and utilities.

Discovery and resolution:

- Searches `task_root_dirs` in order (default: `src/lib/` → `./` → `gro/dist/`)
- `gro foo` looks for `foo.task.ts` in search path, user tasks override builtins
- `gro gro/foo` directly calls builtin, bypassing local overrides
- `gro some/dir` lists all tasks in that directory
- Absolute paths (starting with `/`) and explicit relative paths (`./`, `../`)
  skip search
- Implicit relative paths (`foo/bar`) go through search path

Key features:

- Lazy loading - only imports the invoked task and its dependencies
- Zod schemas - optional `Args` property for validation, types, and
  auto-generated `--help`
- Composition - use `invoke_task` helper (respects overrides) or direct imports
- Args forwarding - CLI args like `gro dev -- vite --port 3000` forward to
  external commands
  - Multiple sections: `gro dev -- vite --port 3000 -- svelte-kit --debug`
  - Special case for nested Gro tasks: `gro check -- gro test --coverage`
    forwards to test
  - Forwarded args override direct args: `gro test --a 1 -- gro test --a 2`
    results in `{a: 2}`
  - `invoke_task` automatically forwards CLI args to invoked tasks
- Error handling - `TaskError` suppresses stack trace, `SilentError` exits
  silently

Task composition patterns:

- `invoke_task('test', args)` - respects user overrides, better logging,
  auto-forwards args (recommended)
- Direct import - `import {task} from './test.task.js'; await task.run(ctx)` -
  faster, tight coupling

Task context:

```typescript
interface TaskContext<TArgs = object> {
	args: TArgs;
	config: GroConfig;
	svelte_config: ParsedSvelteConfig;
	filer: Filer;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}
```

Example task:

```typescript
import type {Task} from '@fuzdev/gro';
import {z} from 'zod';

export const Args = z.strictObject({
	name: z.string().default('world'),
});

export const task: Task<typeof Args> = {
	summary: 'greets someone',
	Args,
	run: async ({args, log}) => {
		log.info(`Hello, ${args.name}!`);
	},
};
```

Run with: `gro mytask` or `gro mytask --name Alice`

Overriding builtin tasks:

```typescript
export const task: Task = {
	run: async ({invoke_task, args}) => {
		await setupCustomEnvironment();
		await invoke_task('gro/test', args); // call builtin
		await teardownCustomEnvironment();
	},
};
```

### Node loader

[Implementation: src/lib/loader.ts, src/lib/register.ts](src/lib/loader.ts)

Custom Node module loader enabling direct execution of TypeScript and Svelte
files without compilation step. Registered via `gro run foo.ts` or
`node --import @fuzdev/gro/register.js foo.ts`.

Capabilities:

- TypeScript files (`.ts`) via Node's experimental type stripping
  (`--experimental-strip-types`)
- Svelte components (`.svelte`) with SSR compilation using `compile()`
- Svelte runes in TypeScript (`.svelte.ts`) using `compileModule()`
- JSON imports (any extension with `type: 'json'` import attribute)
- Raw text imports (`.css`, `.svg`, or `?raw` suffix)

SvelteKit module shims: Best-effort shims for tasks/tests/servers, not identical
to actual SvelteKit modules:

- `$lib/*` → resolved via svelte.config.js alias to `src/lib/`
- `$env/static/public` → reads `PUBLIC_*` vars from `.env`
- `$env/static/private` → reads all vars from `.env`
- `$env/dynamic/public` → `process.env` with `PUBLIC_*` filtering
- `$env/dynamic/private` → full `process.env`
- `$app/environment` →
  `{dev: true, browser: false, building: false, version: ''}`
- `$app/paths` → `{base: '', assets: ''}` from svelte.config.js

### Code generation

[Full documentation: src/docs/gen.md](src/docs/gen.md)

Convention-based codegen system where files containing `.gen.` in their name
export a `gen` function or config object. The `gro gen` task finds these files,
runs them, and writes output to the filesystem.

Naming convention:

- `foo.gen.ts` → outputs `foo.ts`
- `foo.gen.html.ts` → outputs `foo.html`
- Multiple extensions stripped: `.gen.` is removed, last `.ts` dropped
- Custom output via `{content: '...', filename: 'custom.ts'}`
- Can return array for multiple output files

Return values:

- String: default filename, auto-formatted with Prettier
- Object: `{content, filename?, format?}` for control
- Array: multiple files from one genfile
- `null`: no-op

Dependencies: By default, genfiles regenerate when they or their imports change.
Customize via `dependencies` property:

- `'all'` - regenerate on any file change
- `{patterns: [/\.json$/], files: ['package.json']}` - static patterns/files
- Function returning config - dynamic based on which file changed

When it runs:

- `gro gen` - manual trigger
- `gro dev` - watch mode, throttled queue via `gro_plugin_gen`
- `gro build` - one-time generation (assumes fresh, verified by CI)
- `gro sync` - before syncing package.json
- `gro gen --check` - verify no drift (used by `gro check` and CI)

Gen context:

```typescript
interface GenContext {
	config: GroConfig;
	svelte_config: ParsedSvelteConfig;
	filer: Filer;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
	origin_id: PathId; // Same as import.meta.url in path form
	origin_path: string; // origin_id relative to root dir
	changed_file_id: PathId | undefined; // Only during dependency checking
}
```

Example genfile:

```typescript
import type {Gen} from '@fuzdev/gro';

export const gen: Gen = async () => {
	const routes = await findRoutes();
	return `export const ROUTES = ${JSON.stringify(routes)};`;
};

// Multiple outputs:
export const gen: Gen = async () => [
	{content: 'export const foo = 1;', filename: 'foo.ts'},
	{content: '{"version": 1}', filename: 'data.json'},
];

// With dependencies:
export const gen: GenConfig = {
	generate: () => 'generated content',
	dependencies: {
		patterns: [/\.json$/],
		files: ['package.json'],
	},
};
```

### Plugin system

[Full documentation: src/docs/plugin.md](src/docs/plugin.md)

Plugins customize `gro dev` and `gro build` workflows with three lifecycle
hooks: `setup`, `adapt`, and `teardown`. They're objects with a `name` and
optional async functions for each phase.

Lifecycle:

- `setup` - runs first in both dev and build
- `adapt` - runs second, build only (for SvelteKit adapters)
- `teardown` - runs last, build only or dev with `--no-watch`

Dev vs build:

- `gro dev` - creates plugins with `{dev: true, watch: true}`, runs setup, keeps
  process alive
- `gro dev --no-watch` - runs setup → teardown, exits
- `gro build` - creates plugins with `{dev: false, watch: false}`, runs setup →
  adapt → teardown

Plugin interface:

```typescript
interface Plugin<TPluginContext extends PluginContext = PluginContext> {
	name: string;
	setup?: (ctx: TPluginContext) => void | Promise<void>;
	adapt?: (ctx: TPluginContext) => void | Promise<void>;
	teardown?: (ctx: TPluginContext) => void | Promise<void>;
}

interface PluginContext<TArgs = object> extends TaskContext<TArgs> {
	dev: boolean;
	watch: boolean;
}
```

Builtin plugins:

- `gro_plugin_gen` - watches files, queues genfiles when they or dependencies
  change
- `gro_plugin_sveltekit_app` - runs `vite dev` or `vite build` for SvelteKit
  frontends ([docs](src/docs/gro_plugin_sveltekit_app.md))
- `gro_plugin_sveltekit_library` - runs `svelte-package` to publish from
  `src/lib/` ([docs](src/docs/gro_plugin_sveltekit_library.md))
- `gro_plugin_server` - runs Node servers with auto-restart on changes

### Configuration

[Full documentation: src/docs/config.md](src/docs/config.md)

Optional `gro.config.ts` at project root exports `CreateGroConfig` function or
config object. If absent, uses default config from
`src/lib/gro.config.default.ts`.

Default config behavior: Auto-detects project type by checking filesystem:

- `svelte.config.js` → enables `gro_plugin_sveltekit_app`
- `svelte.config.js` + `@sveltejs/package` in package.json + `src/lib/` → enables `gro_plugin_sveltekit_library`
- `src/lib/server/server.ts` → enables `gro_plugin_server`
- Always enables `gro_plugin_gen`

Config interface:

```typescript
interface GroConfig {
	plugins: PluginsCreateConfig; // Function returning array of plugins
	map_package_json: PackageJsonMapper | null; // Hook for package.json automations
	task_root_dirs: Array<PathId>; // Where to search for tasks
	search_filters: Array<PathFilter>; // Exclude patterns for discovery
	js_cli: string; // Node-compatible CLI (default: 'node')
	pm_cli: string; // npm-compatible CLI (default: 'npm')
}
```

map_package_json: Runs during `gro sync` to auto-generate `"exports"` field in
package.json using wildcard patterns for files in `src/lib/`. Return `null`
to opt out.

Example config:

```typescript
import type {CreateGroConfig} from '@fuzdev/gro';

const config: CreateGroConfig = async (base_config) => {
	// Extend default plugins
	const base_plugins = base_config.plugins;
	base_config.plugins = async (ctx) => {
		const plugins = await base_plugins(ctx);
		return [...plugins, myCustomPlugin()];
	};

	// Customize package.json automation
	base_config.map_package_json = (pkg) => {
		pkg.exports = {'.': './dist/index.js'};
		return pkg;
	};

	return base_config;
};

export default config;
```

## File conventions

- Task files: `*.task.ts` in `src/lib/` (or configured `task_root_dirs`)
- Gen files: `*.gen.*` anywhere in `src/` (pattern: `.gen.` substring)
- Test files: `*.test.ts` anywhere (run by Vitest)
- Config: `gro.config.ts` at project root
- SvelteKit config: `svelte.config.js` at project root

Exclusions (configurable via `search_filters`):

- Dot-prefixed directories (`.git`, `.svelte-kit`, `.gro`)
- `node_modules/` (except `node_modules/@*/gro/dist/`)
- Build directories: `.svelte-kit/`, `build/`, `dist/` (except in Gro's own
  directory)

## Builtin tasks

[Complete list: src/docs/tasks.md](src/docs/tasks.md)

- Development: `dev`, `test`, `gen`, `format`, `lint`, `typecheck`
- Production: `build`, `check`, `publish`, `deploy`, `release`
- Utilities: `clean`, `sync`, `run`, `changeset`, `commit`, `reinstall`,
  `resolve`, `upgrade`

Key tasks:

- `dev` - start dev server with watch mode (SvelteKit + Vite via plugins)
  ([docs](src/docs/dev.md))
- `test` - run Vitest tests matching `.test.` pattern ([docs](src/docs/test.md))
- `gen` - run code generation ([docs](src/docs/gen.md))
- `build` - production build with intelligent caching (runs plugin lifecycle:
  setup → adapt → teardown) ([docs](src/docs/build.md))
  - Build caching - skips expensive rebuilds using git commit + optional config
    hash ([docs](src/docs/build.md#build-caching))
  - Conservative correctness: dirty workspace forces rebuild and cleans outputs
  - Outputs validated via parallel hashing
    ([implementation](src/lib/build_cache.ts))
  - Cache survives manual `build/` deletion (stored in `.gro/`)
  - Force rebuild: `gro build --force_build`
- `check` - run all checks (test, gen --check, format --check, lint, typecheck)
- `sync` - run gen, update package.json exports, optionally install packages
- `publish` - version with Changesets, publish to npm, push to git
  ([docs](src/docs/publish.md))
- `deploy` - build and force push to git branch (default: `deploy`)
  ([docs](src/docs/deploy.md))
- `run` - execute TypeScript file with Gro's loader

## Architecture patterns and design

Filesystem as API - tasks, genfiles, and tests discovered by naming convention
(`.task.ts`, `.gen.*`, `.test.ts`). No registration needed - creating a file
makes it available.

Lazy loading - only invoked tasks and their dependencies get imported. Running
`gro` lists all tasks but doesn't execute their code.

Conventions over configuration - file naming patterns define behavior. Minimal
config files (optional `gro.config.ts`). Default config auto-detects project
type by inspecting filesystem.

User overrides - local `src/lib/foo.task.ts` takes precedence over
`gro/dist/foo.task.js`. Call builtin explicitly with `gro gro/foo`. All of Gro's
internals exported from `$lib` for reuse.

Plugin lifecycle - setup initializes, adapt handles production finalization
(SvelteKit adapters), teardown cleans up. Watch mode skips teardown to keep
processes alive.

Filer - central filesystem tracker in task/plugin context. Watches files in dev
mode, tracks dependencies between modules, used by gen plugin to trigger
regeneration.

Timings - performance tracking API.
`const timing = timings.start('name'); await work(); timing();` logs duration.

Minimal abstraction - thin layer over tools (Vite, esbuild, Vitest), forwarding
args and exposing internals. TypeScript everywhere (tasks, config, genfiles).

## Implementation reference

Core systems (src/lib/):

- CLI and task invocation: `gro.ts`, `invoke.ts`, `task.ts`, `invoke_task.ts`,
  `run_task.ts`, `input_path.ts`
- Code generation: `gen.ts`, `gen.task.ts`, `run_gen.ts`, `gen_helpers.ts`
- Plugins: `plugin.ts`, `gro_plugin_*.ts` (gen, sveltekit_app,
  sveltekit_library, server)
- Config: `gro_config.ts`, `gro.config.default.ts`
- Loader: `loader.ts`, `register.ts`

Key utilities:

- Filesystem: `filer.ts`, `paths.ts`
- Module handling: `modules.ts`, `format_file.ts`, `package_json.ts`, `args.ts`
- SvelteKit integration: `svelte_config.ts`, `sveltekit_shim_*.ts`,
  `esbuild_plugin_sveltekit_*.ts`
- Build tools: `esbuild_helpers.ts`, `esbuild_plugin_svelte.ts`,
  `build_cache.ts`

Documentation: [Complete index at src/docs/README.md](src/docs/README.md) - Core
topics: [task](src/docs/task.md), [gen](src/docs/gen.md),
[plugin](src/docs/plugin.md), [config](src/docs/config.md),
[test](src/docs/test.md) | Workflows: [dev](src/docs/dev.md),
[build](src/docs/build.md), [deploy](src/docs/deploy.md),
[publish](src/docs/publish.md)
