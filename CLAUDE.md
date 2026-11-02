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

- TypeScript files (`.ts`) via Node's type stripping
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
- `gro_plugin_deno_server` - runs a Deno server beside the Vite dev server, unbundled

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
	build_cache_config_hash: string; // hash of `build_cache_config`, which normalization drops
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
collides with the Node `gro` on `PATH`.

[src/docs/runtime.md](src/docs/runtime.md) is the hub; its index lists the
runtime docs. A new runtime doc joins the index, and a new term joins
[src/docs/glossary.md](src/docs/glossary.md). What `fgro run` and `fgro eval`
do, their errors and exit codes, the resolver's rules, the type strip, and the
box live there and in each crate's module docs; this section holds what an
agent editing `crates/` needs beside them.

Crates:

- `gro_runtime` - the JS host: a deno_core `JsRuntime` with `deno_webidl`,
  `deno_web`, and gro's bootstrap extension; the resolver, the loader, the
  engine, the box (`confine/`), and `fgro eval`'s protocol (`eval/`). Start
  at its `lib.rs`
- `gro_cli` - the `fgro` binary (`run`, `eval`, `--version`); its `main.rs`
  maps the commands to their docs
- `gro_sys` - the raw system calls the safe std API lacks (descriptors, the
  environment, `RLIMIT_DATA`, the default thread stack, `wait4`, process
  groups, `_exit`), each behind a safe API; Linux only
- `gro_v8` - the V8 boundary: the calls rusty_v8 exposes only as `unsafe`,
  behind a safe API (`HeapReader`); `v8` is pinned to the exact version
  `deno_core` re-exports, so a `deno_core` upgrade moves the pin with it
- `gro_bench` - the measurement harness, independent of the runtime crates;
  its `lib.rs` lists its modules and suites
- `xtask` - dev automation (`cargo xtask`): `fetch-v8`, `build`, `check`,
  `check-rust`, `check-js`, `bench`; its `main.rs` has the commands and exit
  codes

Invariants that live in the code:

- **`unsafe`** only in `gro_sys` and `gro_v8`. Every crate takes the workspace
  lints with `unsafe_code` forbidden; those two re-declare the whole table
  with it at `deny` (Cargo replaces a crate's table whole), and an xtask unit
  test (`crates/xtask/src/lints.rs`) checks each equals the workspace's but
  that and `clippy::undocumented_unsafe_blocks`. Each `unsafe` block is
  allowed by its function's `#[expect]` and explained by a `// SAFETY:`
  comment. `deno_core::scope!` and the `v8` scope macros hide `unsafe` inside
  dependency macros, outside the lint; eval uses them for handle scopes.
- `crates/gro_runtime/js/bootstrap.js` is embedded and must be 7-bit ASCII,
  or the build fails.
- The resolver (`resolver/`) is a pure module: no deno_core types, reading
  only through the `SourceTree` trait. The loader owns the admitted roots
  (`Roots`) and passes them into `resolve_from`. Failures are enums (the
  resolver's `ResolveErrorKind`, the loader's `FailureReason`), never parsed
  from a message; eval's error kind comes from the step that failed and the
  graph's record.
- Type state: `confine::Prepared` → `Prepared::confine` → `Confined`, or
  `Confinement::Unconfined` named by `fgro run`, `make-snapshot`, and unit
  tests; `engine::init_engine` takes a `Confinement` and returns an `Engine`,
  which `run_main` and `make_snapshot` require, and `init_confined_engine`
  takes a `Confined` and returns the `ConfinedEngine` `eval::eval` requires.
  No runtime starts before the process is confined or named unconfined; a
  confined engine whose JIT disagrees with the box's is
  `EngineError::JitConflict`.
- One thread until the box: `fgro eval` keeps its process single-threaded
  until it's confined, in the step order written once, in `confine_eval`'s
  doc in `crates/gro_cli/src/eval.rs`. Every other command runs on one thread
  sized `gro_runtime::STACK_SIZE`.
- `bootstrap::extensions` drops every lazy and synthetic ES module the
  extensions declare: deno_core falls back to a lazy ES module of the exact
  name when the loader refuses an import, and dispatches a synthetic one
  before asking the loader, so either would load into user code past the
  resolver's refusal of `ext:`. `gro_runtime`'s tests import every `ext:`
  specifier the build knows and expect the refusal.
- `bootstrap::REFUSED_OPS` lists the ops eval refuses; a test pins every
  registered op name in `crates/gro_runtime/tests/op_names.txt`, so a
  `deno_core`/`deno_web` upgrade that adds an op fails until it's reviewed.
- `gro_runtime` reads no environment variable but the `phases` feature's
  `GRO_PHASES_FILE` and the `prices` feature's `GRO_PRICES_OFF`; `fgro`
  decides color and passes it in. The `phases` and `prices` features (on
  `gro_cli`, enabling `gro_runtime`'s) are never enabled in release or
  snapshot builds; without them nothing reads the variables.
- A change to the seccomp allowlist reruns the census
  ([src/docs/eval.md](src/docs/eval.md#changing-the-allowlist)).

**Tests.** Confined eval runs only from a snapshot build, so
`crates/gro_cli/tests/eval/`'s tests that evaluate are ignored in a plain `cargo test` and run from
`cargo xtask build --verify`, beside `tests/confine.rs` (no libtest harness,
whose threads would refuse the box). They need Landlock ABI 6 and fail below
it, naming the kernel's ABI. `gro_sys`'s tests that close descriptors, clear
the environment, cap memory, or signal run in a child process, harness off.

**V8 archive, fetch first.** Every cargo build links the prebuilt V8 static
library at the path `.cargo/config.toml` sets as `RUSTY_V8_ARCHIVE`, so the
`v8` build script never downloads one. Plain cargo copies whatever file is at
that path, unchecked, and doesn't recopy it when the file changes. On a fresh
checkout run `cargo xtask fetch-v8` (or any xtask that builds) before plain
cargo; it verifies the pinned SHA-256 (`crates/xtask/src/v8_archive.rs`).
`.cargo/config.toml` also links with `-z pack-relative-relocs`; `RUSTFLAGS`
or `CARGO_ENCODED_RUSTFLAGS` in the environment replaces its flags and drops
it.

**Commands.** The toolchain is pinned in `rust-toolchain.toml`.

- `cargo xtask check` - the one local gate: `check-rust`, `check-js` (the
  JS/TS under `crates/`, which `gro check` doesn't reach), then `gro check`.
  CI runs `check-rust` and `check-js` as its own jobs, so the steps live in
  the xtask only (`crates/xtask/src/check.rs`)
- `cargo xtask build [--release] [--verify]` - the snapshot `fgro`, built in
  two stages, at `<target>/gro_snapshot/<profile>/fgro`; `--verify` adds
  clippy over the `snapshot` feature, the integration tests against that
  build, the size budgets, and a run with the build tree hidden
  (`unshare -rm`, which needs unprivileged user namespaces). A plain
  `cargo build` makes an `fgro` with no snapshot that runs only from its
  build tree and can't `eval`

**Benchmarks.** `gro_bench` times runtimes × workloads in fresh processes,
interleaved, with an A/A noise row, and appends one JSON line per run to
`crates/gro_bench/history.jsonl`. Suites live in `crates/gro_bench/suites/`
(`baseline`, `eval`, `prices`, `memory`; described in gro_bench's `lib.rs`).
Flags are in `crates/xtask/src/bench/mod.rs`, exit codes in
`crates/xtask/src/main.rs` and `crates/xtask/src/error.rs`.

- `cargo xtask bench <suite.json>` - a wall run; `--counters` counts
  instructions and cycles under `perf stat`, `--phases` records per-phase
  durations from a `phases` build; `--samples N` overrides the count
- a suite's `budgets` cap `fgro`'s executable and snapshot sizes; a run over
  one fails before sampling (exit `2`)
- `cargo xtask bench report` - each row's latest record against the previous
  and the last attested one
- Record a suite on a quiet machine, after `cargo xtask build --release` (so
  the bench's own build is a cache hit):
  `cargo xtask bench crates/gro_bench/suites/baseline/suite.json --attest-quiet`
  (or `suites/eval/suite.json`)

**The perf rule.** A change to `gro_runtime` or `gro_cli` runs
`cargo xtask bench crates/gro_bench/suites/baseline/suite.json --counters --base <merge-base>`
and the same for `crates/gro_bench/suites/eval/suite.json` before commit. It
builds the release `fgro` at that revision in a clone outside the checkout,
runs each `fgro` row beside the base copy interleaved, prints instruction,
cycle, and wall medians against the base's, and records nothing. An
instruction move past 2% needs a written reason in review.

## Implementation reference

Core systems (src/lib/):

- CLI and task invocation: `gro.ts`, `invoke.ts`, `args.ts`, `task.ts`,
  `task_registry.ts`, `invoke_task.ts`, `run_task.ts`
- Gen input paths: `input_path.ts`
- Code generation: `gen.ts`, `gen.task.ts`, `run_gen.ts`, `gen_helpers.ts`
- Plugins: `plugin.ts`, `gro_plugin_*.ts` (gen, sveltekit_app,
  sveltekit_library, server, deno_server)
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
