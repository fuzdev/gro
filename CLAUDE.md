# Gro architecture reference

> Task runner and toolkit extending SvelteKit - conventions over configuration,
> filesystem as API

Gro (`@fuzdev/gro`) is a dev tool for building TypeScript projects with
SvelteKit, providing: convention-based task runner, Node loader for
TypeScript/Svelte/SvelteKit modules, code generation system, plugin
architecture, and integrations with Vite, esbuild, Vitest, tsv, ESLint, and
Changesets. It's designed around conventions and the filesystem, not
configuration files.

## Scope

Gro is a **devDependency** - it runs at build time, not in production bundles.

Key responsibilities:

- Task runner and CLI (`gro dev`, `gro build`, `gro test`, etc.)
- Code generation system (`.gen.ts` files)
- Node loader for TypeScript/Svelte without compilation step
- Plugin system for dev/build workflows
- library metadata loading (`library_load.ts`) - analyzes TypeScript/Svelte
  source via `svelte-docinfo` to produce `LibraryJson` consumed by fuz_ui's API
  documentation system, cached in `.gro/` keyed by git commit and revalidated
  against the cache format version and the installed `svelte-docinfo` version;
  on a cache miss it refuses (throws) a repo that extends `$app/tsconfig`
  without having been synced (`sveltekit_tsconfig_is_unsynced` in
  `sveltekit_helpers.ts`), since its external types would analyze as `any`

## Core systems

### Task runner

[Full documentation: src/docs/task.md](src/docs/task.md)

Tasks are TypeScript modules with `.task.ts` suffix that export a `task` object
with a `run` function. Builtins run by name and project tasks by path, with no
search, and each gets a context with args, config, logger, timings, and
utilities.

Resolution (`invoke_task.ts`, `resolve_task`; the CLI's forms in `invoke_cli`):

- `gro foo` runs only the builtin `foo`; a name never means a project task, so
  nothing shadows a builtin, and an unknown name errors with the builtin list
  and the hint `gro task src/lib/foo.task.ts` (one stat says whether it exists)
- `gro task <path>` runs a task file: relative to the cwd (the project root) or
  absolute, `./` optional, `.task.ts`/`.task.js` required, a regular file; the
  task is displayed and keyed by its normalized root-relative path; a flag
  before the path is an error (it would take the path as its value)
- An input containing `/` or ending in a task suffix is a path, anything else a
  builtin name; `invoke_task` takes either by the same rule
- Listing is explicit: `gro` prints the builtins and usage from the generated
  `GRO_BUILTIN_TASK_SUMMARIES` (importing no task module), `gro task` lists the
  project's tasks under `src/`, `gro task some/dir/` lists that directory
- A path without `gro task` errors with the hint, and the removed `gro/` prefix
  errors naming the plain builtin; a path without a task suffix
  (`gro src/x/update`, `gro task src/x/update`) is hinted by what exists
  (`to_task_path_hint`: `update.task.ts`/`.task.js` to run, the directory to
  list, or nothing; a forwarded `-- gro <path>` names only the task file)

Key features:

- Lazy loading - only imports the invoked task and its dependencies
- fuz_schema schemas - optional `Args` property for validation, types, and
  auto-generated `--help`
- Composition - use the `invoke_task` helper (a builtin name or a task path) or
  direct imports
- Args forwarding - CLI args like `gro dev -- vite --port 3000` forward to
  external commands verbatim (`to_forwarded_argv`, never parsed and
  reserialized)
  - Multiple sections: `gro check -- eslint --cache -- tsc --pretty`
  - A task's own flag (`lint`'s `--max-warnings 0`, `tsc`'s `--noEmit`,
    configured `svelte-package` options) is dropped when the forwarded text
    names it (`has_forwarded_flag`); the text follows Gro's args, except
    `--noEmit` goes last (`tsc --build` must be first)
  - Implicit forwarding (`gro test -- --watch`) reads only the first section,
    unless it starts with `gro` or the tool's own name, and only for the task
    the CLI invoked
  - A literal `--` can't be forwarded, except to `gro run`'s script:
    `gro run <path> ...rest` passes everything after the path verbatim, and a
    flag before the path other than `--help` is an error; like `node`, it
    prints no timings and the CLI exits with the script's code, or `128` plus a
    signal's number (a `TaskError`'s `exit_code`, 1–255, read from the
    CLI-invoked task's failure in `invoke.ts`)
  - Special case for nested Gro tasks: `gro check -- gro test --coverage`
    forwards parsed args to test, and `-- gro task src/lib/x.task.ts --a` to a
    project task by its normalized path; `-- gro <path>` and `-- gro gro/x`
    error with the CLI's hints
  - Forwarded args override direct args: `gro test --a 1 -- gro test --a 2`
    results in `{a: 2}`
  - `invoke_task` automatically forwards CLI args to invoked tasks
- Error handling - `TaskError` suppresses stack trace, `SilentError` exits
  silently

Task composition patterns:

- `invoke_task('test', args)` or `invoke_task('src/lib/x.task.ts', args)` -
  better logging, auto-forwards args (recommended)
- Typed names and args - `InvokeTask` checks a literal against `TaskRegistry`
  (`task_registry.ts`) and args against the input type of the task module's
  exported `Args` schema; builtins come from the generated `GroBuiltinTasks`
  (`gro_builtin_tasks.gen.ts`), a project's own tasks, keyed by root-relative
  path, from a genfile exporting `gen_task_registry` (augments `ProjectTasks`);
  a literal task path not in the registry, or a name widened to `string`, takes
  loose `Args`
  ([docs](src/docs/task.md#types-for-invoke_task))
- Direct import - `import {task} from './test.task.ts'; await task.run(ctx)` -
  faster, tight coupling

Task context:

```typescript
interface TaskContext<TArgs = object> {
	args: TArgs;
	config: GroConfig;
	svelte_config: Promise<ParsedSvelteConfig>;
	graph: ModuleGraph; // the project's module graph, loaded on first use
	watcher: Watcher; // a plain file watch, loaded on first use
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}
```

Example task:

```typescript
import type {Task} from '@fuzdev/gro';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import {meta} from '@fuzdev/fuz_schema/meta.ts';

export const Args = s.object({
	name: s.with_default(meta(s.string, {description: 'who to greet'}), 'world'),
});
export type Args = s.Infer<typeof Args>;

export const task: Task<Args> = {
	summary: 'greets someone',
	Args,
	run: async ({args, log}) => {
		log.info(`Hello, ${args.name}!`);
	},
};
```

Run with: `gro task src/lib/mytask.task.ts` or
`gro task src/lib/mytask.task.ts --name Alice`

Wrapping a builtin is composition, not an override: a project task calls the
builtin by name, and the project's script points at the wrapper's path.

```typescript
export const task: Task = {
	run: async ({invoke_task, args}) => {
		await setupCustomEnvironment();
		await invoke_task('test', args); // always the builtin
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
- Svelte runes in JS and TypeScript modules (`.svelte.ts`, and `.svelte.` as an infix
  like `.svelte.test.ts`, vite-plugin-svelte's rule) using `compileModule()`
- JSON imports (any extension with `type: 'json'` import attribute)
- Raw text imports (`.css`, `.svg`, or `?raw` suffix)

Svelte config: read from the options passed to the `sveltekit()` plugin in
`vite.config.ts` (SvelteKit 3 has no `svelte.config.js`), awaited only inside the
`load` hook, which most invocations never reach. The `resolve` hook needs no
config: package.json subpath imports (`#lib/*`, `#routes/*`) resolve through
Node, and SvelteKit's deprecated `alias` option isn't supported (Gro warns when
it's set).

Strict specifiers: a relative or absolute import resolves to exactly the file it
names (`resolve_specifier.ts`), matching the extension rules of the runtime's
[resolver](src/docs/resolver.md). Extensions are mandatory, nothing is probed,
and `.js` never maps to `.ts`; a specifier naming no file is refused with the
specifier, the importer, and the fix, carrying Node's `code`
(`ERR_UNSUPPORTED_DIR_IMPORT` for a directory, else `ERR_MODULE_NOT_FOUND`) and
the path's `url`, so `import.meta.resolve` returns the URL as in Node while the
import still fails. A path specifier is `./`, `../`, `/`, `.`, or `..`
(`is_path_specifier`), shared by the loader, the esbuild filter, and the Filer,
so `'.hidden.ts'` is bare, as in Node. The esbuild plugins for server builds
follow the same rule, and the Filer's import graph records the exact path a
specifier names (`to_specifier_path_id`), existing or not, so a file created
later links to its importers; imports from inside `node_modules` keep Node's and
esbuild's own resolution.

SvelteKit module shims: Best-effort shims for tasks/tests/servers, not identical
to actual SvelteKit modules:

- `$app/env/public` / `$app/env/private` → the vars declared in `src/env.ts`
  (`defineEnvVars`), resolved from `.env` files plus `process.env` and
  validated against their schemas when the module runs (not inlined)
- `$app/env` → `{dev: true, browser: false, building: false, version}` with
  `version` from the config's `version.name`
- `$app/paths` → `resolve`/`asset` prefixed with the config's `paths.base`/`paths.assets`
- `$app/forms`, `$app/navigation`, `$app/state` → no-op stubs

The loader resolves these to its own sibling modules, so they work with a
global Gro in a project that doesn't install it.

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

- String: default filename, auto-formatted with tsv
- Object: `{content, filename?, format?}` for control
- Array: multiple files from one genfile
- `null`: no-op

Inputs: in `gro dev`, genfiles regenerate when they or their imports change.
The static `inputs` property declares the other files they read:

- `'all'` - regenerate on any change the dev watch reports
- `{patterns: ['*.json'], files: ['src/lib/template.html']}` - gitignore patterns
  anchored at the project root (matched by tsv's `IgnoreStack`, files inside the
  root only) and exact paths (anywhere); only changes under `watch.dir` (default
  `src/`) trigger

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
	svelte_config: Promise<ParsedSvelteConfig>;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
	origin_id: PathId; // Same as import.meta.url in path form
	origin_path: string; // origin_id relative to root dir
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

// With inputs:
export const gen: GenConfig = {
	generate: () => 'generated content',
	inputs: {
		patterns: ['*.json'],
		files: ['src/lib/template.html'],
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

- `gro_plugin_gen` - watches files, queues genfiles when they, their imports, or their inputs
  change
- `gro_plugin_sveltekit_app` - runs `vite dev` or `vite build` for SvelteKit
  frontends ([docs](src/docs/gro_plugin_sveltekit_app.md))
- `gro_plugin_sveltekit_library` - runs `svelte-package` to publish from
  `src/lib/`, then rewrites relative `.ts`→`.js` import specifiers in the
  emitted `.d.ts`/`.svelte` ([docs](src/docs/gro_plugin_sveltekit_library.md))
- `gro_plugin_server` - runs Node servers with auto-restart on changes

### Configuration

[Full documentation: src/docs/config.md](src/docs/config.md)

Optional `gro.config.ts` at project root exports `CreateGroConfig` function or
config object. If absent, uses default config from
`src/lib/gro.config.default.ts`.

Default config behavior: Auto-detects project type from `package.json` and the
filesystem, deferred until plugins are created:

- `@sveltejs/kit` in package.json deps or dev deps (not peer deps) → enables
  `gro_plugin_sveltekit_app`
- `@sveltejs/package` in package.json + the lib directory → enables
  `gro_plugin_sveltekit_library`
- `server/server.ts` in the lib directory → enables `gro_plugin_server`
- Always enables `gro_plugin_gen`

Config interface:

```typescript
interface GroConfig {
	plugins: PluginsCreateConfig; // Function returning array of plugins
	map_package_json: PackageJsonMapper | null; // Hook for package.json automations
	search_filters: Array<PathFilter>; // Exclude patterns for discovery
	js_cli: string; // Node-compatible CLI (default: 'node')
	pm_cli: string; // npm-compatible CLI (default: 'npm')
	watch: WatchOptions; // what `graph` and `watcher` watch (default: `src/`, no filter)
}
```

map_package_json: Runs during `gro sync` to auto-generate `"exports"` field in
package.json using wildcard patterns for files in `src/lib/`. Return `null`
to opt out. Every `internal/` directory is blocked at any depth by a null
exports entry (`"./internal/*": null`, `"./domain/internal/*": null` — Node's
explicit-exclusion form; exports keys allow one `*`, hence one key per
outermost internal directory, whose `*` covers anything nested below), so
internal modules ship in dist for public modules to import
but can't be imported by consumers (the `internal/` convention;
`svelte-docinfo` honors the same signal in discovery and excludes
`internal/` from analysis at any depth by default), and internal files
don't count toward which wildcard patterns are emitted.

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

- Task files: `*.task.ts`, run by path (`gro task` lists those under `src/`)
- Gen files: `*.gen.*` anywhere in `src/` (pattern: `.gen.` substring of the file name)
- Test files: `*.test.ts` anywhere (run by Vitest)
- Config: `gro.config.ts` at project root
- Vite config: `vite.config.*` at project root - the Svelte config is read through it

Exclusions (configurable via `search_filters`, which test paths relative to
the project root, so a project under `~/.cache/` still searches):

- Dot-prefixed directories (`.git`, `.svelte-kit`, `.gro`)
- `node_modules/`
- Build directories: `build/`, `dist/`, `dist_server/`

## Builtin tasks

[Complete list: src/docs/tasks.md](src/docs/tasks.md)

- Development: `dev`, `test`, `gen`, `format`, `lint`, `typecheck`
- Production: `build`, `check`, `publish`, `deploy`, `release`
- Utilities: `clean`, `sync`, `run`, `changeset`, `commit`, `reinstall`,
  `upgrade`

Key tasks:

- `dev` - start dev server with watch mode (SvelteKit + Vite via plugins)
  ([docs](src/docs/dev.md))
- `test` - run Vitest tests matching `.test.` pattern ([docs](src/docs/test.md))
- `gen` - run code generation ([docs](src/docs/gen.md))
- `build` - production build with intelligent caching (runs plugin lifecycle:
  setup → adapt → teardown) ([docs](src/docs/build.md))
  - Build caching - skips expensive rebuilds using git commit + optional config
    hash + Gro and Node versions + a hash of `node_modules/.package-lock.json`
    ([docs](src/docs/build.md#build-caching))
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
`gro` lists the builtins from generated summaries without importing any task;
`gro task` imports the project's tasks to read their summaries.

Conventions over configuration - file naming patterns define behavior. Minimal
config files (optional `gro.config.ts`). Default config auto-detects project
type by inspecting filesystem.

No shadowing - a name always means a builtin and a path always means a file,
so a local `src/lib/foo.task.ts` runs as `gro task src/lib/foo.task.ts` and
never replaces `gro foo`. Gro's modules in `src/lib` are exported for reuse,
except those under `internal/`.

Plugin lifecycle - setup initializes, adapt handles production finalization
(SvelteKit adapters), teardown cleans up. Watch mode skips teardown to keep
processes alive.

Module graph and watcher - the task/plugin context's `graph` (`ModuleGraph`:
project modules, transitive dependents, content-based change events; the gen
plugin uses it to trigger regeneration) and `watcher` (`Watcher`: a plain file
watch; the server plugin uses it to rebuild). The interfaces live in
`module_graph.ts`, types only; the Node implementations in `module_graph_node.ts`
import the Filer and chokidar on first use, so tasks that never use them never
load them. The Filer family (`internal/filer.ts`, `internal/disknode.ts`,
`internal/watch_dir.ts`, `internal/parse_imports.ts`) is internal, not exported.
A chokidar error (like `EACCES` reading a directory) is a warning through the
graph's and watcher's `log` (`console` with none), not fatal; the Filer stats before reading, so a
directory or special file (a FIFO) is a contentless node, and the graph
forwards a change whose read finds no file as one `delete` (`#remove` notifies
only a node that had contents); a file reached by a bare specifier (a package's,
even when linked) is a contentless node, never read, so its imports aren't
edges (its importer's edge is kept), while a file a path or `#` subpath import
reaches is read.

Timings - performance tracking API.
`const timing = timings.start('name'); await work(); timing();` logs duration.

Minimal abstraction - thin layer over tools (Vite, esbuild, Vitest), forwarding
args and exposing internals. TypeScript everywhere (tasks, config, genfiles).

## Rust workspace (experimental)

Beside the npm package, `crates/` holds a Cargo workspace for gro's own JS
runtime, a Rust binary embedding V8. The crates are unpublished and outside the
npm package's `files`. During development the binary is `fgro`, so it never
collides with the Node `gro` on `PATH`. The README introduces the runtime in
one plain paragraph and links out. [src/docs/runtime.md](src/docs/runtime.md)
is the hub: an index of the runtime docs under its status block, then the
thesis, why, the security model, the design, open questions, and the roadmap.
The rest each own one subject: [src/docs/run.md](src/docs/run.md) is the
reference for the built binary (what loads, the globals, errors, exit codes),
[src/docs/eval.md](src/docs/eval.md) the reference for `fgro eval` (the driver,
determinism, the limits, the serializer, the envelope and its error kinds),
[src/docs/resolver.md](src/docs/resolver.md) the resolver's rules and
departures from Node, [src/docs/typescript.md](src/docs/typescript.md) the
type-strip rules, and [src/docs/comparison.md](src/docs/comparison.md) the
comparison with Node, Deno, and Bun, and [src/docs/glossary.md](src/docs/glossary.md)
defines the terms once (runtime.md's terms list summarizes the core ones).
A new runtime doc joins runtime.md's index, and a new term joins the glossary.

- `gro_runtime` - the JS host, a minimal web-standards runtime on V8: a
  deno_core `JsRuntime` with `deno_webidl` and `deno_web`, gro's bootstrap
  extension (installs the pure web globals, removes `Deno` and `__bootstrap`;
  its `crates/gro_runtime/js/bootstrap.js` is embedded and must be 7-bit ASCII,
  or the build fails), and the strict resolver as a pure module (`resolver/`,
  no deno_core types; it reads only through a `SourceTree` trait and returns
  a record of the URL, the file's identity, the root the module is inside and
  its origin, and the `package.json` files it consulted): relative and
  absolute specifiers with mandatory extensions, and
  bare specifiers through a package's `"exports"` (Node's algorithm, with the
  conditions `gro`, `import`, `default` matched in the package's key order),
  found by self-reference or in `node_modules` up from the importer's real
  path. A module's origin is a function of its real path over the graph's
  admitted roots (`Roots`: the entry's project root and each package root a
  bare import resolved into, innermost by component winning, none with a
  `node_modules` segment below it), which the loader owns and passes into
  `resolve`; the loader refuses a root admitted around a module already
  recorded in an outer one (`RootConflict`), and `import.meta.resolve`
  admits nothing. The loader keeps the run's `Graph` (`loader/graph.rs`):
  with `Recording::Off`, what `fgro run` uses, only the roots and each URL's
  root and file, the minimum loading needs; with `Recording::Full`, what
  `fgro eval` uses, also each
  import edge and how it was reached, each loaded module's format and the
  blake3 of the bytes read, the union of the `package.json` files consulted
  (the entry's included), and each failure as data (resolve or load, with
  its reason as an enum, never parsed from a message), kept per site in the
  order each first happened. The file loaded is the file resolved: the lookup takes its
  `FileId` (device, inode, and birth time, by `symlink_metadata` on the real
  path, so a last component swapped for a symlink is refused), the loader
  records it per URL (the first record stands; a re-resolve changes nothing),
  and `resolver::read_checked` opens the path with `O_NONBLOCK` (so a FIFO
  swapped in can't block), refuses an open file with another identity, and
  reads from that open file; `SourceTree::read` checks a `package.json` the
  same way. Not caught (accepted): a directory swapped between `canonicalize`
  and the identity read, and a file remade where the filesystem records no
  birth time or within the same tick of its clock as the original's birth,
  which falls back to device and inode, numbers ext4 reuses. Deliberate departures from Node: origin
  containment (a relative or absolute specifier resolves only inside its
  importer's root, never into `node_modules` below it or into another
  admitted root nested in it, and `"exports"` targets and self-references
  stay inside their package, all checked on the real path; an entry inside
  `node_modules` runs as its package, and one in no package is refused), the
  `node_modules` search is bounded by the project root, found first (the
  first `package.json` outside `node_modules` that isn't a type marker, whose
  keys are all `"type"` or `"sideEffects"`, failing closed, so npm and yarn
  workspaces aren't supported), and a module with no project root gets no
  bare imports except a self-reference, which resolves through its own
  package's `"exports"` and stays inside its package; a world-writable
  directory (the other-write bit) ends every walk up, with no
  `package.json` or `node_modules` in it read,
  and a package found through one (its `node_modules`, `@scope`, root, a
  linked root's real holders, or a linked `node_modules`'s or `@scope`'s
  real parent) is refused (`SourceTree::is_world_writable`); a package with
  no `"exports"` is refused (no `main`), a `.js`
  file needs `"type": "module"` with no format sniffing, and `.ts` inside
  `node_modules` is stripped. `.js`/`.mjs` load as written and `.ts`/`.mts`
  load stripped of their types in process by tsv (`tsv_ts`, a git dependency
  pinned by rev; the loader in `loader/` blanks types so positions never
  move, and refuses TypeScript with runtime semantics, such as an `enum`, with
  its position and a fix); schemes, `#` imports, CommonJS, `.cts`, `.tsx`,
  declaration files, and Svelte modules (vite-plugin-svelte's rule: `.svelte.`
  then any segments then `.js` or `.ts`) are refused with the reason, and a
  fix where there is one.
  V8's process-wide setup is `engine::init_engine` (`EngineOptions`: V8's
  entropy drawn from `getrandom`, always, an optional ICU default locale,
  and two V8 flags set before V8 starts, `jitless` and
  `disallow_code_generation_from_strings`, both off for `fgro run`), once
  per process before the first runtime, and it takes the process's
  `confine::Confinement`: `Confined`, made only by
  `confine::Prepared::confine`, or `Unconfined`, which `fgro run`,
  `make-snapshot`, and the unit tests name. `confine` (Linux only; elsewhere
  it refuses) checks `/proc/self/status` reads `Threads: 1` (rereading while
  a joined thread finishes exiting) and then, on that one thread, installs
  the box: `no_new_privs`, Landlock (the `landlock` crate, ABI 6 required,
  `HardRequirement` and full enforcement checked: read files and directories
  beneath the box's roots only, by the handles the pre-pass opened, every
  other filesystem right ABI 6 knows handled and so denied, TCP bind and
  connect denied, the signal and
  abstract unix socket scopes; below ABI 6, without Landlock, or with it
  off, it refuses naming the kernel's ABI, with no weaker mode), then two
  `seccompiler` filters with no thread sync, one killing the process on
  `KILLED_CALLS` (exec, ptrace, the cross-process memory calls, bpf,
  perf_event_open, userfaultfd, kexec, io_uring, the module calls), one
  refusing with `EACCES` `REFUSED_CALLS` (every `socket`, `connect`,
  `bind`, and `listen`, the calls that change a file's mode, owner, times,
  extended attributes, or attribute flags, which Landlock at ABI 6 doesn't
  gate, the ones `libc` doesn't name on every architecture by their number
  in the table x86_64 and aarch64 share from 424 on, `inotify_add_watch`
  and `fanotify_init`, the System V IPC calls, and `mq_open` and
  `mq_unlink`), every `fcntl` in `REFUSED_FCNTLS` (`F_SETLEASE`, whose
  lease stalls another process's open for writing and tells the holder,
  and `F_NOTIFY`; the advisory locks, `flock` and `F_SETLK`, pass: they
  write nothing, and only hold off another process locking the same file
  inside the box while the eval runs), every `ioctl` whose request isn't in
  `KEPT_IOCTLS` (`FIONBIO`, `FIOCLEX`, `FIONCLEX`, `FIONREAD`, `TCGETS`,
  `TIOCGWINSZ`, compared as the kernel's 32-bit request; every other,
  setting attribute flags, the generation, extended attributes, verity, or a
  filesystem's own among them, is refused), every call in `SELF_ONLY_CALLS`
  (setting resource limits, priority, I/O priority, CPU affinity, or
  scheduling) that names a process other than the caller (its pid or who
  argument not 0, or for `setpriority` and `ioprio_set` its which not one
  process), which Landlock's scopes don't reach (`getpriority`,
  `ioprio_get`, and the `sched_get*` calls pass), and every `socketpair` but
  `KEPT_SOCKETPAIR` (a unix stream pair, for tokio; a datagram pair could
  `sendto` a socket file by path, which Landlock below ABI 9 doesn't gate);
  an architecture `seccompiler` can't target is
  `ConfineError::SeccompArch`; `ConfineError::is_host_unsupported` splits
  eval's `environment` from `internal`. A confined engine's entropy callback
  aborts with a message when `getrandom` fails, rather than let V8 fall
  back to the clock (`/dev/urandom` is outside the box);
  it returns an `Engine` token naming both, which `run_main`,
  `snapshot::make_snapshot`, and `eval::eval` require, so no runtime starts
  on an engine nobody chose or before the process is confined or named
  unconfined (V8's workers start with the engine); each runtime's own choices are
  `RuntimeOptions`, the console's included (`ConsoleOptions`: which stream
  each level writes to and whether each is colored, read when the run
  starts, never in the snapshot). The crate reads no environment variable;
  `fgro` decides color.
  It starts from a startup snapshot when the embedder passes one
  (`snapshot.rs`; built in its own process from the same extension list, with
  the lazy scripts it didn't load embedded beside it as `residual.rs`), else
  from source, reading `deno_web`'s JS from the build tree; the per-run
  steps run after startup either way. `bootstrap::extensions` drops every
  lazy and synthetic ES module the extensions declare (`deno_web`'s
  `locks.js` and `webtransport.js`): deno_core falls back to a lazy ES module
  of the exact name when the loader refuses an import, and dispatches a
  synthetic one before asking the loader, so either would load into user
  code past the resolver's refusal of `ext:`. `eval/` is `fgro eval`'s
  protocol in two steps: `eval::prepass` resolves the entry
  (`resolver::resolve_entry_with_bound`, which also says how the project
  root was found, `EntryBound`), refuses one inside `node_modules` or with no
  project root as a usage `resolve`, and computes the read roots
  (`confine::Prepared::for_project`: the project root and the link closure,
  each `node_modules` entry and `@scope` entry whose real path is a
  directory outside the roots so far, holding none, holding a `package.json`
  that is a file (not a symlink), and held by no world-writable directory
  (the resolver's `SourceTree::is_world_writable`), followed through each
  new root's `node_modules` to a fixed point, directory structure only, each
  root opened as it's found, `O_PATH` with `O_DIRECTORY` and `O_NOFOLLOW`,
  and kept only when the kernel names the handle by the root's path, so the
  box goes up on the directories found); then `eval::eval` (which always
  returns an `Envelope`) runs it, the loader's `Rules::box_roots` checking
  every module and `package.json` read against those roots first (a graph
  can reach past them, through a link the closure doesn't follow, in a
  nested `node_modules` or a subdirectory's own, or drops, to a directory
  holding the project or another package's, or a `package.json` symlinked
  out, so a refusal is recorded as the loader's breach and fails
  the eval as `internal`, naming the path, the box, and the fix). A
  confined `eval::eval` refuses a build with no startup snapshot, whose
  runtime would read `deno_web`'s JS from the build tree, as `internal`
  (`eval::no_snapshot_error`, which `fgro eval` returns after its pre-pass
  and before the box, so the refusal is the same on every host). The engine
  `eval::engine_options` gives (jitless unless `--jit`, code generation from
  strings off, locale `en-US`, checked against the `Engine` token), the
  console to stderr uncolored, the stdin driver evaluated first as a side
  module at the synthetic URL `gro:driver`, then the entry, then
  `driver.default(namespace)` or the export, the strict host serializer
  (`eval/serialize.rs`: an iterative walk over V8 values in Rust, on a
  stack of its own, in batches each in its own handle scope, inside V8's
  `DisallowJavascriptExecutionScope`, reading property descriptors so no
  getter or `toJSON` runs, refusing non-plain objects, cycles, lone
  surrogates, and nesting past 1000 with the JS path), the drain, and the
  driver's optional `after_drain`; the error kind comes from the phase that
  failed plus the first static failure the graph recorded, never from a
  message. Eval's runtime starts from the same snapshot as `run`'s and is
  made eval's per run (`Mode::Eval`, never frozen into the snapshot): the
  bootstrap's start reads `op_gro_deny` and installs `determinism: deny`
  (`js/bootstrap.js`'s `deny`: the clock, randomness, and machine reads and
  `AbortSignal.timeout` throw a refusal naming the operation and reason,
  each replaced where it lives; `WeakRef`, `FinalizationRegistry`, and
  `console.time*` inert; the four timer globals, `Blob`, `File`, and the
  object-URL functions deleted, since a `File` keeps the clock and a blob
  part a random id in their own state) before the driver;
  the op middleware `bootstrap::refuse_ops` gives each op in
  `REFUSED_OPS` (the timer, clock, memory, and blob ops, `op_eval_context`,
  `op_compile_function`, `op_import_sync*`) the slow-only
  `op_gro_refused`'s implementation, and a test pins every registered op
  name (`crates/gro_runtime/tests/op_names.txt`); the isolate refuses
  `Atomics.wait` and caps V8's heap, not array-buffer memory
  (`eval/limits.rs`: the near-heap-limit callback stops the run and raises the cap once, and an out-of-memory
  handler, the one process static besides the engine's, writes a prepared
  `heap` envelope through a handle opened before the run and exits 1); a
  watchdog thread stops the run at `--timeout`, and the serializer checks
  for a stop once per batch; a time zone other than UTC (by `Intl`'s id and
  `new Date(0).toString()`, so an alias named otherwise fails too) fails as
  `environment` before the driver; `heap` and `timeout` come from the stop
  recorded, never a message, and an `error.hint` names `--jit` or that
  code generation is off. The loader's `Rules` carry eval's refusals: every `import()`
  before anything resolves (the entry graph's listed in the envelope's
  `access.dynamic`), and every import from the driver ("a stdin driver is
  import-free"), each recorded as its own `FailureError`. `deno_core::scope!` and the `v8`
  scope macros hide `unsafe` inside dependency macros, outside the
  `unsafe_code` lint; eval uses them to open handle scopes on the runtime.
  `gro_runtime`'s tests import
  every `ext:` specifier the build knows (deno_core's own, and every module
  and script the extensions declare) statically, by `import()`, and by
  `import.meta.resolve`, and expect the refusal
- `gro_cli` - the `fgro` binary: `fgro run <file>` runs an ES module (JS or
  TypeScript) from disk and takes only the entry file, with no script
  arguments yet (exit `0` success, `1` a run that fails, `2` a usage error,
  including an entry that doesn't resolve; the full table is in
  [src/docs/run.md](src/docs/run.md)); console output is colored per stream,
  only on a terminal, unless `FORCE_COLOR` decides it or `NO_COLOR`,
  `NODE_DISABLE_COLORS`, or `CLAUDECODE` set non-empty, or `TERM=dumb`, turns it off
  (the order is in [src/docs/run.md](src/docs/run.md)). `fgro eval <module>
  [--driver -] [--export NAME] [--jit] [--timeout MS]` prints one envelope on stdout and
  exits by the same table (`2` only for a usage error or an entry that
  doesn't resolve or that no project owns); it reads stdin for the driver and no environment
  variable ([src/docs/eval.md](src/docs/eval.md)). Every
  command but `eval` runs on one thread with a 32 MiB stack of its own
  (`gro_runtime::STACK_SIZE`, tsv's `STACK_SIZE`), keeping a debug build inside
  the 16 MiB its strip is sized for (a release build needs 2 MiB); a module
  past tsv's depth limits is a load error, not an abort. `eval` runs on the
  main thread, which reads stdin, the working directory, and the
  out-of-memory handle, runs `eval::prepass` on a sized thread it joins,
  confines the process alone, then starts the engine and evaluates on a
  second sized thread, so every thread V8 and tokio start comes after the
  confinement. Confined eval runs only from a snapshot build, so
  `tests/eval.rs`'s tests that evaluate are ignored in a plain `cargo test`
  and run from `cargo xtask build --verify`, beside `tests/confine.rs`
  (no libtest harness, whose threads would refuse the box: each case
  confines a child of its own and reports what it could reach); on a kernel
  whose Landlock is below ABI 6 they fail, naming its ABI.
  `gro_runtime::run_main` documents the requirement for any embedder.
  `fgro --version` names the git commit it was built from (with `-dirty`, or
  `unknown` for a plain `cargo build`), the V8 version, and whether a snapshot
  is embedded. Its `snapshot` feature embeds a startup snapshot read from
  `GRO_SNAPSHOT_DIR`, and only that feature reads `GRO_COMMIT`;
  a plain `cargo build` builds it without one, so it runs only from its build
  tree
- `gro_bench` - the measurement harness, independent of the runtime crates:
  `cargo xtask bench <suite.json>` times every runtime × workload row in fresh
  processes, interleaved, with an A/A noise row, and appends one JSON line per
  run to `crates/gro_bench/history.jsonl`. The baseline suite
  (`crates/gro_bench/suites/baseline/`) times Node (also with its compile cache
  on), Deno, and Bun on startup scripts and on fuz_util's TS module graph, cold
  and warm, and `fgro` on both (it has no on-disk cache, so its cold and warm
  graph rows run the same command); its workload weights are provisional. The
  graph is fuz_util's collection in a sibling `corpora` checkout
  (`fuzdev/corpora`), pinned by tree id. A workload that names a `collection`
  runs from a fresh copy of it, written from the pinned tree's objects into
  the gitignored `target/bench/`, with the workload's script at the copy's
  root so its relative imports stay inside the collection; the command still
  runs from the suite's directory. A suite naming
  `fgro` gets the release snapshot build: the bench builds it first and
  refuses any other binary, then prints each `fgro` row against its gate row
  (warm rows against `node_compile_cache`, cold ones against `node`, `deno`
  beside), report only. Record it on a quiet machine with
  `cargo xtask bench crates/gro_bench/suites/baseline/suite.json --attest-quiet`,
  after `cargo xtask build --release`, so the bench's own build is a cache hit
  and doesn't load the machine just before timing
- `xtask` - dev automation (`cargo xtask`): `fetch-v8`, `build`, `check`,
  `check-rust` (the Rust half alone, which CI's rust job runs), `check-js`
  (the JS/TS step alone, which CI's Node job runs), `bench`

**V8 archive, fetch first.** Every cargo build links the prebuilt V8 static
library at the path `.cargo/config.toml` sets as `RUSTY_V8_ARCHIVE` (under
`target/rusty_v8/`, named by version), so the `v8` build script never
downloads one. Plain cargo copies whatever file is at that path, unchecked,
and doesn't recopy it when the file changes, only when the path does. On a
fresh checkout, run `cargo xtask fetch-v8` (or any xtask that builds) before
plain cargo: it downloads the archive if it's missing and checks its recorded SHA-256
(`crates/xtask/src/v8_archive.rs`, which also fails when `Cargo.lock`'s `v8`
version moves off the pin), and replaces a file that doesn't match after
`cargo clean -p v8`, so the next build recopies and relinks. Before running V8
code, the xtask's builds also check the library copy the `v8` build script
last wrote (`<target>/<profile>/gn_out/obj/librusty_v8.a`, shared by its
build units), a stale-copy check that names the remedy. The pin protects the
download's integrity; local writes to the target directory are out of scope,
since anyone who can write there can alter any build output already. A plain
cargo build against a bad file, fixed by hand before any xtask runs, goes
undetected until a newer `v8` verifies `RUSTY_V8_ARCHIVE_SHA256` itself. CI
runs the fetch first, as the first step of `check-rust`.

**Snapshot builds.** `cargo xtask build [--release]` makes the `fgro` that
starts from a snapshot and runs off its build tree, in two stages: the plain
`fgro` writes `snapshot.bin` and `residual.rs` with its internal
`make-snapshot` subcommand, then `fgro` is rebuilt with the `snapshot`
feature and `GRO_COMMIT`, the checkout's `HEAD` with `-dirty` when the tree
differs (untracked files count, the bench history doesn't), read just before
that build and embedded with `option_env!` for `--version`; the result is
`<target>/gro_snapshot/<profile>/fgro` (`<target>` is
cargo's target directory, as `cargo metadata` reports it). `--verify` adds
clippy over the feature's code, the integration tests against that build, and
a run of a fixture with the cargo registry's sources and gro's own extension
sources hidden (tmpfs mounts under `unshare -rm`, which needs unprivileged user
namespaces and fails when they're unavailable).

`cargo xtask check` is the one local command for both halves: `check-rust`
(the V8 archive, the Rust gates `cargo fmt`, `clippy`, `test`, and `doc` with
warnings denied, then `build --verify`), `check-js` (the JS/TS under
`crates/`, which `gro check`'s typecheck and format don't reach), then
`gro check`. CI runs the first two as its own jobs, so the gate list lives in
the xtask only. The toolchain is pinned
in `rust-toolchain.toml`, every crate takes the workspace lints, and `unsafe`
is forbidden.

## Implementation reference

Core systems (src/lib/):

- CLI and task invocation: `gro.ts`, `invoke.ts`, `args.ts`, `task.ts`,
  `task_registry.ts`, `invoke_task.ts`, `run_task.ts`
- Gen input paths: `input_path.ts`
- Code generation: `gen.ts`, `gen.task.ts`, `run_gen.ts`, `gen_helpers.ts`
- Plugins: `plugin.ts`, `gro_plugin_*.ts` (gen, sveltekit_app,
  sveltekit_library, server)
- Config: `gro_config.ts`, `gro.config.default.ts`
- Loader: `loader.ts`, `register.ts`

Key utilities:

- Filesystem: `module_graph.ts`, `module_graph_node.ts`, `internal/filer.ts`, `paths.ts`
- Module handling: `modules.ts`, `format_file.ts`, `package_json.ts`, `args.ts`
- SvelteKit integration: `svelte_config.ts`, `sveltekit_shim_*.ts`,
  `esbuild_plugin_sveltekit_*.ts`
- Build tools: `esbuild_helpers.ts`, `esbuild_plugin_svelte.ts`,
  `build_cache.ts`, `build_cache_dependencies.ts`

Documentation: [Complete index at src/docs/README.md](src/docs/README.md) - Core
topics: [task](src/docs/task.md), [gen](src/docs/gen.md),
[plugin](src/docs/plugin.md), [config](src/docs/config.md),
[test](src/docs/test.md) | Workflows: [dev](src/docs/dev.md),
[build](src/docs/build.md), [deploy](src/docs/deploy.md),
[publish](src/docs/publish.md)
