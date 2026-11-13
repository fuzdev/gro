# Gro architecture reference

> security-first JS runtime and toolchain on V8 - today a task runner and toolkit
> extending SvelteKit, conventions over configuration, filesystem as API

Gro (`@fuzdev/gro`) is a dev tool for building TypeScript projects with
SvelteKit, providing: convention-based task runner, Node loader for
TypeScript/Svelte/SvelteKit modules, code generation system, plugin
architecture, and integrations with Vite, Vitest, tsv, ESLint, and
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

[src/docs/task.md](src/docs/task.md) owns the behavior: resolution, listing,
`Args`, `TaskContext`, `invoke_task` and its types, args forwarding, `gro run`,
errors and exit codes. Where it lives:

- Resolution - `resolve_task` in `invoke_task.ts`; the CLI's forms in
  `invoke_cli`. A name means only a builtin, a path only a file; an input with
  `/` or a task suffix is a path (`invoke_task` takes either by the same rule)
- Hints for a path without `gro task` or without a task suffix -
  `to_task_path_hint` (`internal/task_hints.ts`)
- Listing - `gro` prints the builtins from the generated
  `GRO_BUILTIN_TASK_SUMMARIES` (importing no task module); `gro task` imports
  the project's tasks under `src/` to read their summaries
- Args forwarding - `to_forwarded_argv` (verbatim, never parsed and
  reserialized) and `has_forwarded_flag` (a forwarded flag replaces a task's
  own) in `args.ts`; CLI-forwarded args override args passed to
  `invoke_task` (`gro test --a 1 -- gro test --a 2` → `{a: 2}`). The functions
  take the CLI args after the CLI's own path (`process.args`), and a root or
  `invoke_task`'s `raw_rest_args` option passes them
- Exit code - `to_exit_code` in `invoke.ts` reads a `TaskError`'s `exit_code`
  (how `gro run` passes on its script's code) from the CLI-invoked task's
  failure
- Errors - `TaskError` suppresses the stack, `SilentError` exits silently;
  `is_task_error` / `is_silent_error` (`task.ts`) match subclasses and other
  copies of Gro, where `instanceof` misses
- Typed `invoke_task` - `InvokeTask` checks literals against `TaskRegistry`
  (`task_registry.ts`); builtins from the generated `GroBuiltinTasks`
  (`gro_builtin_tasks.gen.ts`), project tasks from a genfile exporting
  `gen_task_registry(create_fs_paths(fs))` (augments `ProjectTasks`)
  ([docs](src/docs/task.md#types-for-invoke_task))
- Fs boundary - `invoke_task` takes the root's `FsPaths` as `fs_paths` in
  `InvokeTaskOptions`, built by the CLI entry (`invoke.ts`), and `run_task`
  passes it to the tasks a task invokes; it's never on `TaskContext`, so a task
  that reaches fs imports the host module itself
- Wrapping a builtin is composition: a project task calls it by name and the
  `package.json` script points at the wrapper's path; nothing shadows a builtin

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
  (types stripped by tsv first)
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
(`is_path_specifier`), shared by the loader and the Filer,
so `'.hidden.ts'` is bare, as in Node. The Filer's import graph records the exact
path a specifier names (`to_specifier_path_id`), existing or not, so a file
created later links to its importers; imports from inside `node_modules` keep
Node's own resolution.

SvelteKit module shims: Best-effort shims for tasks and tests, not identical
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

[src/docs/gen.md](src/docs/gen.md) owns the behavior: naming, return values,
`GenContext`, `inputs`, when gen runs, and `gro gen --check`. Where it lives:

- Task and runner - `gen.task.ts`, `run_gen.ts`, `gen.ts`
- `inputs` - `gen_helpers.ts`, patterns matched by tsv's `IgnoreStack`; input
  paths in `input_path.ts`
- Dev regeneration - `gro_plugin_gen`, a throttled queue over the task
  context's `graph`
- After editing a genfile, a task's `summary`, or anything a genfile reads, run
  `gro gen`; `gro check` fails on drift (`gro gen --check`). `gro build` and
  `gro sync` run gen too (`--no-gen` skips).

### Plugin system

[src/docs/plugin.md](src/docs/plugin.md) owns the lifecycle
(`setup` → `adapt` → `teardown`), the `Plugin` and `PluginContext` interfaces,
and error handling; `plugin.ts` implements them. Builtin plugins:

- `gro_plugin_gen` - reruns genfiles when they, their imports, or their inputs
  change
- `gro_plugin_sveltekit_app` - `vite dev` / `vite build`
  ([docs](src/docs/gro_plugin_sveltekit_app.md))
- `gro_plugin_sveltekit_library` - `svelte-package`, then `.ts`→`.js`
  specifier rewrites in the emitted `.js`/`.d.ts`/`.svelte`
  ([docs](src/docs/gro_plugin_sveltekit_library.md))
- `gro_plugin_server` - a server entry beside the Vite dev server, run with
  Gro's loader and restarted through `spawn_restartable` when the entry or its
  imports change in the module graph (`internal/server_restarts.ts`)
  ([docs](src/docs/gro_plugin_server.md))

### Configuration

[src/docs/config.md](src/docs/config.md) owns `gro.config.ts`, `GroConfig`
and `CreateGroConfig`, the default config's project detection, and each option
(`map_package_json` and the `internal/` null-exports rule, `search_filters`,
`watch`, `build_cache_config`). Where it lives:

- Types and loading - `gro_config.ts` (`cook_gro_config` hashes
  `build_cache_config` and drops the raw value; `load_gro_config` loads and
  cooks)
- Default config - `gro.config.default.ts`; detection is deferred until `dev`
  or `build` creates plugins (`has_sveltekit_app` / `has_sveltekit_library` in
  `sveltekit_helpers.ts`)
- Exports automation - `package_json_sync` in `package_json.ts`, run by
  `gro sync`

## File conventions

- Task files: `*.task.ts`, run by path (`gro task` lists those under `src/`)
- Gen files: `*.gen.*` anywhere in `src/` (pattern: `.gen.` substring of the file name)
- Test files: `*.test.ts` anywhere (run by Vitest)
- Config: `gro.config.ts` at project root
- Vite config: `vite.config.*` at project root - the Svelte config is read through it

Exclusions (configurable via `search_filters`, which test paths relative to
the project root, or outside it to the searched directory, so a project or a
searched directory under `~/.cache/` still searches):

- Dot-prefixed directories (`.git`, `.svelte-kit`, `.gro`)
- `node_modules/`
- Build directories: `build/`, `dist/`

## Builtin tasks

[Complete list: src/docs/tasks.md](src/docs/tasks.md), generated from the task
files' summaries. Docs per task: [dev](src/docs/dev.md),
[test](src/docs/test.md), [gen](src/docs/gen.md), [build](src/docs/build.md)
(sync, gen, the build cache and its key, post-build verification),
[publish](src/docs/publish.md), [deploy](src/docs/deploy.md); `run` in
[task.md](src/docs/task.md#gro-run). The build cache is `build_cache.ts` and
`build_cache_dependencies.ts`, stored in `.gro/build.json`.

`gro build` with a dirty workspace skips the cache and deletes outputs first;
`--force_build` ignores the cache; a build that started clean fails if it left
the workspace dirty (with `--allow_dirty` it passes, uncached).

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

Module graph - the task/plugin context's `graph` (`ModuleGraph`: project
modules, transitive dependents, content-based change events; the gen plugin uses
it to trigger regeneration). The interface lives in `module_graph.ts`, types
only; the Node implementation in `module_graph_node.ts` imports the Filer and
chokidar on first use, so tasks that never use it never load them. The Filer
family (`internal/filer.ts`, `internal/disknode.ts`, `internal/watch_dir.ts`,
`internal/parse_imports.ts`) is internal, not exported. A chokidar error (like
`EACCES` reading a directory) is a warning through the graph's `log` (`console`
with none), not fatal; a file that doesn't parse warns with its location and
keeps the edges it last parsed with; the Filer stats before reading, so a
directory or special file (a FIFO) is a contentless node, and the graph forwards
a change whose read finds no file as one `delete` (`#remove` notifies only a
node that had contents). A package's file is a contentless node, never read, so
its imports aren't edges, while its importer's edge is kept. That's a file a bare
specifier reaches, even when linked, and one whose path from the watched
directory passes through a `node_modules` directory, judged both from the
directory as named and from its real path, like one a `#` mapping such as
`"#dep": "dep"` resolves to. Any other file a path or `#` subpath import
reaches is read.

Timings - performance tracking API.
`const timing = timings.start('name'); await work(); timing();` logs duration.

Minimal abstraction - thin layer over tools (Vite, Vitest), forwarding
args and exposing internals. TypeScript everywhere (tasks, config, genfiles).

## Rust workspace (experimental)

Beside the npm package, `crates/` holds a Cargo workspace for Gro's own JS
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

- `gro_runtime` - the JS host: a deno_core `JsRuntime` (deno_core patched,
  in `vendor/`) with `deno_webidl`, `deno_web`, and Gro's bootstrap
  extension; the resolver, the loader, the engine, the box (`confine/`), and
  `fgro eval`'s protocol (`eval/`). Start at its `lib.rs`
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
  `check-rust`, `check-js`, `vendor-patch`, `bench`; its `main.rs` has the
  commands and exit codes

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
  `gro_cli`, enabling `gro_runtime`'s) are off in the binary that ships;
  without them nothing reads the variables.
- A change to the seccomp allowlist reruns the census
  ([src/docs/eval.md](src/docs/eval.md#changing-the-allowlist)).

**Tests.** Confined eval runs only from a snapshot build, so
`crates/gro_cli/tests/eval/`'s tests that evaluate are ignored in a plain `cargo test` and run from
`cargo xtask build --verify`, beside `tests/confine.rs` (no libtest harness,
whose threads would refuse the box). They need Landlock ABI 6 and fail below
it, naming the kernel's ABI. `gro_sys`'s tests that close descriptors, clear
the environment, cap memory, or signal run in a child process, harness off.

**Vendored crates.** `vendor/` holds patched copies of published crates
(`deno_core`, patched for deterministic snapshots), wired through
`[patch.crates-io]` and outside the workspace and its checks (their own rustc
warnings print but fail nothing). [vendor/README.md](vendor/README.md) is the
index of every patch and the upgrade procedure, so a `deno_core` upgrade
re-vendors it per the README; edit vendored code only as a marked
`// gro patch:` ([vendor/CLAUDE.md](vendor/CLAUDE.md)). `check-rust` proves
each tree is its published crate plus its patch, and
`cargo xtask vendor-patch <name>` regenerates a patch. With the patch, the
same sources, toolchain, and cargo home build the same `fgro`, byte for byte,
from any checkout.

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
  the snapshot remade by that `fgro` and compared byte for byte, clippy over
  the `snapshot` feature, the integration tests against that
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
  sveltekit_library, server)
- Config: `gro_config.ts`, `gro.config.default.ts`
- Loader: `loader.ts`, `register.ts`

Key utilities:

- Filesystem: `module_graph.ts`, `module_graph_node.ts`, `internal/filer.ts`, `paths.ts`
- Host modules: `fs.ts`, the fs host module's Node implementation
  ([docs](src/docs/host_modules.md)); `fs_error.ts`, its `FsError` and `is_fs_error`,
  grant-free; `fs_search.ts`, the recursive search over a handle;
  `fs_paths.ts`, `FsPaths`, the boundary where a path from outside (a CLI arg, config, a
  well-known path) becomes a handle, its symlinks followed, built at a composition root with
  `create_fs_paths(fs)` and passed down, its machinery in `internal/fs_boundary.ts` with
  `is_protected_dir` guarding every removal; library code takes an `FsPaths` or a narrow
  handle as a parameter, and only roots (tasks, `invoke.ts`, genfiles, the default config,
  plugin modules, the rendered `$app/env/*` shims) import `fs`, as they import `process.ts`,
  the process host module, library code taking a `cwd` string, a plain record, or the CLI args,
  but for `internal/fs_boundary.ts`, which reads `HOME` once at load for its protected directories;
  Gro's library and task code uses no `node:child_process` but `spawn.ts`, and
  no `node:fs` but the Node-only machinery
  [host_modules.md](src/docs/host_modules.md#where-to-import-them) lists;
  `src/test/host_module_declarations.test.ts`
  fails if a host module's emitted declarations name a type outside the ES libs
  and its short list of web types (`WEB_TYPES`), such as a Node or DOM-only type
- CLIs: `cli.ts` - `find_cli(spawn, name)` tries the project's `node_modules/.bin` and then
  `PATH`, resolved through a `Spawn`, so only roots import the spawn host module, as for `fs`
- Network: `net.ts`, the net host module, `net.fetch` and `narrow` bound to hosts, following redirects itself so each `Location` is checked; `net_error.ts`, its `NetError` and `is_net_error`, grant-free ([docs](src/docs/host_modules.md#net))
- Watch: `watch.ts`, the watch host module, `watch.dir(handle, {recursive, signal})` an async iterable of `add`/`update`/`delete`/`rescan` events with paths relative to the handle, symlinks never followed, buffered to `max_pending`, over chokidar imported on first call and apart from the Filer's `internal/watch_dir.ts`; its errors are `FsError` ([docs](src/docs/host_modules.md#watch))
- Git: `git.ts`, library code over a `git` `SpawnProgram` a root builds with `to_git_program`;
  a read of the local repo captures with `out`, anything that may prompt uses `run` (or `out` with
  `stdin: 'inherit'` when its output is needed), a caller's
  branch or tag name passes `assert_git_ref_name` so it's never a refspec or pattern,
  and an origin goes after `--end-of-options`, while `git_pull` refuses one beginning with `-`
- Long-lived children: `spawn_restartable.ts`, restarts over one `SpawnProgram`
- Module handling: `modules.ts`, `format_file.ts`, `package_json.ts`, `args.ts`
- SvelteKit integration: `svelte_config.ts`, `sveltekit_shim_*.ts`
- Build tools: `build_cache.ts`, `build_cache_dependencies.ts`

Documentation: [Complete index at src/docs/README.md](src/docs/README.md) - Core
topics: [task](src/docs/task.md), [gen](src/docs/gen.md),
[plugin](src/docs/plugin.md), [config](src/docs/config.md),
[test](src/docs/test.md) | Workflows: [dev](src/docs/dev.md),
[build](src/docs/build.md), [deploy](src/docs/deploy.md),
[publish](src/docs/publish.md)
