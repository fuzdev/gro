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
  own) in `task_args.ts`; CLI-forwarded args override args passed to
  `invoke_task` (`gro test --a 1 -- gro test --a 2` → `{a: 2}`). The functions
  take the CLI args after the CLI's own path (`args`, from `args.ts`), and a root or
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
  extension; the resolver, the loader, the builtin modules, the host ops
  behind them (`host/`), the engine, the box (`confine/`), and `fgro eval`'s
  protocol (`eval/`). Start at its `lib.rs`
- `gro_cli` - the `fgro` binary (`run`, `eval`, `--version`); its `main.rs`
  maps the commands to their docs
- `gro_sys` - the raw system calls the safe std API lacks (descriptors, the
  environment, `RLIMIT_DATA`, the default thread stack, a thread's stack
  start, `wait4`, process groups, `_exit`, and the filesystem calls confined
  beneath a directory descriptor by `openat2`, in `beneath`), each behind a
  safe API; Linux only
- `gro_v8` - the V8 boundary: the calls rusty_v8 exposes only as `unsafe`,
  behind a safe API (eval's recording `ArrayBuffer` allocator,
  `record_array_buffers`); `v8` is pinned to the exact version
  `deno_core` re-exports, so a `deno_core` upgrade moves the pin with it
- `gro_bench` - the measurement harness, independent of the runtime crates;
  its `lib.rs` lists its modules and suites
- `xtask` - dev automation (`cargo xtask`): `fetch-v8`, `build`, `check`,
  `check-rust`, `check-js`, `check-reproducible`, `vendor-patch`, `bench`;
  its `main.rs` has the commands and exit codes

Invariants that live in the code:

- **`unsafe`** only in `gro_sys` and `gro_v8`. Every crate takes the workspace
  lints with `unsafe_code` forbidden; those two re-declare the whole table
  with it at `deny` (Cargo replaces a crate's table whole), and an xtask unit
  test (`crates/xtask/src/lints.rs`) checks each equals the workspace's but
  that and `clippy::undocumented_unsafe_blocks`. Each `unsafe` block is
  allowed by its function's `#[expect]` and explained by a `// SAFETY:`
  comment. `deno_core::scope!` and the `v8` scope macros hide `unsafe` inside
  dependency macros, outside the lint; eval uses them for handle scopes.
- The embedded source (`crates/gro_runtime/js/bootstrap.js`) must be 7-bit
  ASCII, or the build fails (`ascii_str_include!`). A builtin has no source of
  its own: each row's backing script is made from the row, of
  `bootstrap/template.rs`'s literals, which the build holds to ASCII (the
  wrapping `format!` adds, by the test that holds each script). A
  builtin embeds no module of `src/lib`: what it shares with its Node module
  is behavior, held by the differential cases, never source.
- The resolver (`resolver/`) is a pure module: no deno_core types, reading
  only through the `SourceTree` trait. The loader owns the admitted roots
  (`Roots`) and passes them into `resolve_from`. Failures are enums (the
  resolver's `ResolveErrorKind`, the loader's `FailureReason`), never parsed
  from a message; eval's error kind comes from the step that failed and the
  graph's record.
- The builtin table (`resolver/builtin.rs`) is the one source of the builtin
  modules: `&'static` data of gro's own types, each row's specifier, grant,
  and exports. `builtin_rows!` hands the rows to the bootstrap, which names
  each row's backing script and synthetic module from it, and
  `bootstrap/template.rs` makes the backing script from the row, when the
  snapshot is made, into the residual table the binary embeds, or once a
  process when a runtime starts without a snapshot. No snapshot holds a
  backing script, and gro's extension declares no lazy script: `deno_core`
  reads a backing script from its residual table the first time a run
  imports the builtin, and runs it then, so an unimported builtin costs a
  start its facade's name.
  - A backing script is one function in two levels, of one of two
    templates. The outer takes `deno_core`'s bootstrap object, runs once a
    run, and takes the one op, `op_gro_namespace`, from `core.ops`, and, in
    a row with a member that declares an error family
    (`native::hands_over_error`, read from the native members' table:
    `env_error` and `env` today), the realm's `Error` from `primordials`, a
    read that costs a first import, so no other row takes it. It returns
    the inner, which takes the scope of its importer's grant, an argument
    the script's call passes (`template::scope_argument`; `undefined` while
    no grant has one), and asks the host for the namespace. The script
    calls both at once. A first import can come after user code has
    patched the global intrinsics, and the script names no global but
    `undefined` and reads no member of an intrinsic: each read is a
    destructuring of what `deno_core` handed it before any user code ran.
    It hands on nothing but what the op returns, reads no other op, and
    lets no level escape. `template.rs`'s test holds every row's script to
    the two texts byte for byte, the one with `Error` to exactly the rows
    the pinned table (`tests/native_members.txt`) shows declaring a family.
    A row's script reads nothing and decides nothing; the namespace and
    whatever has identity or state are the host's (a member, a class, a
    brand, a table of handles, an exit code), in `OpState`.
  - Every row's members are native (`host/native.rs`): each export, and
    each method of a kind of handle the row makes, a function or a value
    the host makes from `NATIVE_MEMBERS`, the table `native_members!`
    declares, the first time a run asks for the row's namespace. The inner
    level calls `op_gro_namespace`, handing over its scope, and from a row
    that hands it over the realm's `Error`, which the run holds unchecked
    (`ErrorClasses::hand_over`) until a class is first made of it; the op
    refuses an `Error` from any other row. The op
    serves every row and knows the row by the script that calls it, the
    backing script `deno_core` compiled under the row's name from the
    table, never by an argument, so no row's script can ask for another's;
    it returns the namespace the run holds, or makes it and holds it
    (`NativeNamespaces`, which every run's start puts, empty), one per row
    while no row is scoped (an instance per origin keys it by the scope as
    well, the functions and prototypes made once a row): a frozen
    null-prototype record of the exports and `default`, each function
    frozen, named, of the `length` its declared arguments give, and no
    constructor, each value made once with it, data (the arguments'
    frozen array, the process's id, the runtime's version), a handle
    (`env`'s root), or a class (`EnvError`), each kind's prototype a frozen
    null-prototype record of its methods. Nothing is held until the
    namespace is whole. So a start pays one op for every row, and a first
    import no shell's compile. A factory makes its handles of the
    prototype the run holds (`native::prototype`), and a value declared a
    handle of a kind (`handle KIND(prototype)`) is given the one the
    namespace is being made with.
  - An `import()` runs a backing script on the caller's stack, so a first
    `import()` of a builtin with less than `BUILTIN_IMPORT_STACK` left above
    V8's limit has `deno_core` build the module from the event loop instead
    (the loader's `should_load_synthetic_esm`; `loader/headroom.rs`, with
    the measurement behind the amount), and the import settles a turn later:
    no importer can cut a first import short, and no failure of one is
    reported on a stack too short for the report, which could end the
    process. `deno_core` builds one module of a builtin however its loads
    interleave (the `deno_core` patch, `vendor/README.md`), so every import
    gets the one namespace. A backing script throws nothing at load: what
    can fail for a reason of the run's fails in what the module exports, as
    `cwd` does when called.
    Nothing known ends a level early (a first import has the stack it needs,
    an op a level calls throws only for want of stack, and a run never
    terminates V8), and both stay safe to run again all the same: the amount
    is a measurement each build's sweep holds, not a bound, and where the
    stack start isn't read (off glibc) nothing is deferred, so a deep first
    import can be cut short there; `deno_core` answers a failure by keeping
    nothing of the run and running the whole script at the next import
    (`bootstrap.rs`), so a cause found later costs a rerun, never a
    half-made module or an effect done twice. Neither level stores anything
    outside its own bindings: `deno_core` keeps the namespace only once both
    have returned, so nothing half-made is published. State every importer
    shares is the host's, and an op either level calls can be called again.
    `crates/gro_cli/tests/host.rs` starts every row's first import a frame
    apart up from the stack's limit, interleaves deep and shallow first
    imports in a turn, and holds every import to one whole module; nothing
    checks that an op a level calls is safe to repeat.
  - `crates/gro_cli/tests/host.rs` imports every row of the table in a
    realm whose every reachable intrinsic is a trap that reports as it's
    used, and exercises everything the row exports or reaches: each
    function called with no arguments, with hostile ones (a proxy, a proxy of
    a function, an object of getters, a subclassed string and array), with
    each argument the fixture gives for it replaced in turn by a hostile one,
    and with a hostile receiver, each class constructed (one the host made
    among them, told by its `prototype`, which can't be replaced), each
    getter read, each setter given a value, and the same for what a call
    returns or throws. The names a backing script reads, every member's,
    and each export's are trapped on the shared prototypes too, so a read
    of one that an object lacks reports. A hostile argument answers every
    read made of it,
    `cause` included, and what a proxy's trap does by default runs
    unwatched, since it's the caller's code. A function reached fails it
    until the fixture has an entry for it, with the arguments it takes or
    none, and a use of a hostile argument until the test lists it for that
    function, reviewed: only an error class's constructor makes any, the
    conversions and the `cause` read the Node class makes. A call that ends
    the process, as `exit`'s do, is one the test lists with its code: each
    call is reported before it's made, and the run is made again without the
    one it ended in. What a call leaves to run later, a promise's reaction,
    it doesn't watch.
  - An error row's class and guard are the host's: the class
    (`host/error_class.rs`) made natively the first time a run needs it,
    the realm's `Error.prototype` taken from an error V8 makes and its
    `Error` handed over from `primordials` by the backing script of the
    first row a run imports that hands it over, held for the run, and
    checked the first time a class is made of it, so a run that makes no
    class pays for no check, and a value that fails is dropped for the next
    import's; and the class held in `OpState` (`ErrorClasses`) once whole,
    so a first import ended early makes it again and every later need finds
    the one class; the row's namespace holds it as a native value, and its
    guard as a native function (`host/native.rs`), and any member throws an
    instance with one call (`ErrorClasses::throw`). A member declares the
    family whose error it throws (`throws ENV_ERROR(errors)` in
    `native_members!`), and the error row's class the family it is
    (`class ENV_ERROR(family)`); throwing a family's error and making its
    class take the `DeclaredFamily` only the macro makes, from the
    declaration, so a member throws no family it didn't declare and its row
    hands the `Error` over. A family missing from `FAMILIES` throws when its
    class would be made. A start makes none and puts the empty holder only.
    The guard reads data properties only, so it runs no code of a caller's.
  - A handle is the host's (`host/handle.rs`): an object the host makes of
    a prototype, the record of native methods the host made with the row's
    namespace, its state under a private symbol of its kind, the brand,
    then frozen. A method reads the handle it's called on natively and
    refuses anything without the brand (a look-alike, a proxy, another
    row's handle) and decides from the handle's state, never from the run's
    grants, which only gate that the run holds the row at all. `env`'s root
    handle is a value of its namespace, made from the run's grant with it,
    and the one handle whose refusal hints at a grant: a method knows it
    by comparing it with the export the run's namespace holds, which the
    host never hands out (`native::is_held_export`). A handle whose
    state is native Rust holds a number under its brand, naming its state
    in a table in `OpState` (`host/blake3.rs`'s hashers, `Blake3State`,
    which every run's start puts, empty), beside a weak reference to the
    handle whose finalizer drops the state once the handle is collected;
    what makes one takes the scope mutably, as a native member's callback
    is given it, since the weak reference is made of the isolate.
  - The resolver returns a table hit ahead of any package lookup; the loader
    checks it against the run's `Authority` at link time, for a static import
    and `import()` alike (`loader/resolve.rs`): with none (eval's) nothing is
    imported, a grant-free row included, checked before the grant; a
    confined run's (`Authority::Confined`) package module is granted none,
    and a grant-free row links for the project and a package alike; an
    unconfined run (`Authority::Unconfined`, `fgro run -A`) grants every
    module every row, and lifts no refusal but a grant's. A builtin keeps
    no per-importer state.
  - A builtin declares no types: its types are its Node module's.
    `src/test/builtin_types.test.ts` holds the native members' table, as
    `tests/native_members.txt` pins it, to each row's Node module, read
    through the TypeScript compiler, both ways, naming any member on one
    side only: the same exports, each the same kind (a function, data, a
    handle of the same kind, a class), each kind of handle's methods the
    own members of the Node module's interface it's named for (a kind's
    brand is `<specifier>#<Interface>`), and each function and method of a
    `length` its declaration allows: exactly JS's count, to the first
    defaulted or rest parameter, for a function with a body, and for an
    interface's method from its required parameters to all of them, since
    only its implementation says whether an optional one is defaulted.
    Names, kinds, and arity, never argument or result types, reading the
    rows from the table.
    `crates/gro_cli/tests/host.rs` holds every row's values to what its Node
    module printed (`tests/fixtures/host/`, whose
    `expected.json` `node oracle.js --write` regenerates under Node and
    `src/test/host_differential.test.ts` checks against Node).
  - The table also reserves specifiers it doesn't serve (each row's `.js`
    spelling, derived from the row, `UNBUILT_HOST_MODULES`, the error modules
    not built among them, `SPLIT_MODULES`, the modules Gro has none of whose
    members are modules of their own, each refused naming them, and a respelling of these or of a row's specifier,
    read as URL parsing and the path's decoding read an `"exports"` target, in
    `resolver/specifier.rs`), which the resolver refuses ahead of any package
    lookup; a reserved one is data only, never reaching `builtin_rows!`. A
    package's module can't reach a grant through the project's: its bare
    import resolving into the project root is refused
    (`ResolveErrorKind::PackageImportsProject`). The loader's
    `resolve_with_scope` refuses an `import()` in code compiled from a string
    first, then an attribute under any key but `type` for every specifier,
    then any attribute on a builtin before its grant; an `import()`'s
    attributes reach it through the `deno_core` patch (`vendor/README.md`),
    since the runtime's attribute callback, the unpatched route, runs on every
    static import.
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
- `bootstrap::extensions` drops every lazy ES module the extensions declare
  and every other extension's synthetic ES modules, keeping gro's own, one
  per builtin, each named by its `builtin:` URL; gro's own extension
  declares no lazy script, which deno_core would register at every start
  (the builtins' backing scripts are residual ones). deno_core falls back to a
  lazy ES module of the exact name when the loader refuses an import, which
  would load it past the resolver's refusal of `ext:`; and it links a static
  import naming a synthetic module by that raw name, whatever the loader
  resolved the name to (a refusal still fails the import), so a synthetic
  module's name must be one the resolver refuses from source. `gro_runtime`'s
  tests import every `ext:` and `builtin:` specifier the build knows and
  expect the refusal, and pin the kept synthetic names to the table's URLs.
- One grant choke point: the one host op, `op_gro_namespace`, is declared
  through `host::host_ops!`, whose first step is `require_authority` in
  `host/grant.rs`, which refuses only a runtime that holds no `Authority`
  at all, so the op runs in every run and in no eval; it checks the row's
  grant once it knows the row from its calling script
  (`grant::require_grant`), before anything is made. It takes each
  argument as the JS value passed (`v8::Local<v8::Value>`), never through
  an `#[op2]` conversion, which could run the caller's code or truncate:
  its body reads the value natively, after the check, and throws through
  the scope an error V8 makes from the realm's intrinsic
  (`throw_type_error` in `host/mod.rs`), since an op's `Err` is built by a
  JS callback of `deno_core`'s. The run's `Authority` goes into the
  loader and the runtime's `OpState` per run, never into the snapshot or
  the extension list, and eval's runtime takes none. A test fails on an
  op of gro's extension, whatever its name, that's neither on
  `bootstrap::UNGATED_OPS` nor a host op (or that's on an object), and
  another on a host op that runs with no authority. Nothing at compile
  time makes a host op use the macro: the first test is the check.
- The same choke point for native members (`host/native.rs`): each is
  declared through `native_members!`, a function or a value made with the
  namespace, whose callback's or maker's first step is the check,
  `require` of the grant it names or `require_authority`, throwing
  `not_granted` through the scope, and which then hands a function's body
  its receiver and each argument as the JS value passed, never converted.
  The host makes every native function but an error family's class
  (`error_class.rs`, whose constructor checks nothing: it reads no state
  of the run and reaches nothing a grant guards, and no runtime without
  an authority makes one) from the table the macro writes. Tests fail on a
  native function or callback made in the code of the crates linked into
  `fgro` anywhere but `native.rs` and `error_class.rs` (a source scan, by
  the `CallbackArguments` type every native callback names and each API
  that makes one), on a member that runs with no authority, or whose body
  ran before its refusal, on a value made with no authority or without its
  grant, on a member of a granted row (a probe table's)
  that runs without its grant, on a member whose grant isn't its row's, on
  the namespace op answering a script that isn't the row's, on a
  namespace or prototype whose records or functions aren't the table's, or
  aren't frozen, and on a `DeclaredFamily` made anywhere but by the macro
  (a source scan, by its maker's name, and in its own module by every
  line that names it); `tests/native_members.txt` pins the table
  (module, name, place, a function's `length` or a value's kind, grant,
  the family declared) as `op_names.txt` pins the ops.
- `bootstrap::REFUSED_OPS` lists the ops eval refuses, each one the runtime
  registers. Of the ops and classes `deno_web` declares, the runtime registers
  those on `bootstrap::WEB_OPS` and `bootstrap::WEB_CLASSES`, the ones a
  script it can load reads from `core.ops`, and drops the rest before the
  runtime or the snapshot is made (`bootstrap::extensions`). The tests of
  `bootstrap/web_scripts.rs` read the web extensions' scripts the runtime can
  load, and fail on a read that isn't kept, on a kept name no script reads,
  and on a script that reaches `ops` any way but destructuring `core.ops`; a
  runtime test fails on a read that isn't a function. Tests pin every
  registered op name in `crates/gro_runtime/tests/op_names.txt` and every
  dropped one in `tests/dropped_op_names.txt` beside it, so a
  `deno_core`/`deno_web` upgrade that adds an op fails until it's reviewed,
  and every refused one in `tests/refused_op_names.txt`, since the tests of
  the refusals read their names from the list itself.
- A start from the snapshot binds no op again (`skip_op_registration`, set
  in `runtime::new_runtime`): the snapshot's op functions, each without its
  fast call, are the only ones for the run (a builtin's backing script takes
  its op from the bootstrap object, which holds them). `deno_core` would
  make each op that has a fast call again at the first call of
  `op_load_ext_script` or `op_lazy_load_esm`, and nothing calls either once
  a run has started. Each op function reaches the starting runtime's
  implementation of its op, eval's refusal included, and that runtime's
  `OpState`, through the
  external-reference table `deno_core` builds from the runtime's ops. The
  runtime's tests start from a snapshot too, in child processes: one makes
  it (`process_tests::make_startup_snapshot`), and a second starts every
  runtime of its own from it (`runtime::tests::snapshot_child`), since a
  process's isolates share one read-only heap. They hold the op functions,
  eval's refusals, and the host op's check to what a start from source
  gives, and eval's refusals on each of eval's engines, jitless and with
  the JIT, each engine a child's. `cargo xtask build --verify` runs them
  built with the build's profile, with one more
  (`the_built_snapshot_refuses_the_refused_ops`) whose runtimes start from
  the snapshot the build embeds. A snapshot names an op's function by its
  place in the table of the process that made it, and a start reads the
  place in its own table, so that test sees what `fgro` would of an op the
  snapshot encodes by another's place. The test binary is `gro_runtime`'s,
  not `fgro`: the release profile but for `panic = "abort"`, which cargo
  leaves off a test, over another crate graph, so what its link merges
  needn't be what `fgro`'s does.
- No snapshot is made of ops eval's refusals can't land on. A snapshot
  writes an op's function by the first external reference that holds its
  address, a release build merges identical functions into one address,
  and eval puts its refusal at the refused ops' references alone. So
  `snapshot::make_snapshot` fails, naming both ops, when a refused op's
  slow function shares its address with another op's function, unless
  that is a refused op's slow one, or its fast function with another op's
  slow one, and when a name on
  `bootstrap::REFUSED_OPS` is no op's (the module doc of `snapshot.rs`).
  The addresses are the making process's, read through the `deno_core`
  patch: the plain `fgro` of stage one, whose snapshot the build embeds,
  and the snapshot `fgro` when `--verify` makes it again.
- `gro_runtime` reads no environment variable but the `phases` feature's
  `GRO_PHASES_FILE` and the `prices` feature's `GRO_PRICES_OFF`; `fgro`
  decides color and passes it in, as it passes the working directory and
  its own name and version, and the variables `--allow-env=` grants (every
  variable under `-A`), which `gro_cli/src/run.rs` reads once as the run
  starts (`env_vars`, through its `Environment` seam) and hands in as
  `RuntimeOptions::env`. The host asks the OS for the process's id, `pid`,
  when a run first imports the module. The `phases` and `prices` features (on
  `gro_cli`, enabling `gro_runtime`'s) are off in the binary that ships;
  without them nothing reads the variables.
- A run's or an eval's runtime is handed back when it's over
  (`SpentRuntime`, in `RunOutcome` and beside eval's envelope): dropped, it's
  torn down, which is what an embedder gets, and `fgro` forgets it on the
  thread that owns it, since the process exits next. Every command exits
  through `end` in `crates/gro_cli/src/main.rs`, the one exit step, but for
  eval's out-of-memory backstop (`eval/limits.rs`), which ends the process
  from V8's out-of-memory callback without reaching it. `end` takes only a
  code and reaches no runtime. Console output is written and flushed as
  it's printed, so nothing is buffered there today; a later host module
  that buffers stdout or stderr must have it written before the runtime is
  handed back and before `exit` calls the embedder's step, or keep
  the buffer where `end` can reach it.
- `gro_runtime` ends its embedder's process itself in two places only:
  eval's out-of-memory backstop exits 1, and a confined engine whose random
  source fails aborts (`engine.rs`). A program's `exit` doesn't: its
  function calls the step the embedder gave (`RuntimeOptions::exit_step`), from
  inside the call, since stopping V8 doesn't stop a program at once
  (`host/exit.rs` has why); a run that grants `exit` without a step fails
  before it starts (`StartError::NoExitStep`), so the grants stay plain
  data, and an embedder that must outlive its program doesn't grant
  `exit`. `fgro`'s step is `run::exit_now`, which prints what was refused
  and reaches `end`; `run::exit_code` is the one place a run's exit code
  is decided, for a program's code, a failure, and a refusal alike. The
  exit code a program sets is the run's state in `OpState`, never a
  script's, and so is the run's record of refusals
  (`loader::refused::Refusals`), the loader's, which `OpState` holds for
  any op (`loader::RefusalLog`): the `import()`s the loader refuses by
  rule, and each call a native member refuses for a grant its handle
  lacks, recorded by the member as it throws (`env`'s `not_granted`), one
  record listed, cut, and counted alike, so either fails the run caught or
  not; a refusal by `env`'s root also records the grant that would have
  allowed it (`NeededGrant`), from which `fgro` prints its hint. No
  in-process test grants a step that returns: a program that exits is run
  in a child process.
- A change to the seccomp allowlist reruns the census
  ([src/docs/eval.md](src/docs/eval.md#changing-the-allowlist)).

**Tests.** Confined eval runs only from a snapshot build, so
`crates/gro_cli/tests/eval/`'s tests that evaluate are ignored in a plain `cargo test` and run from
`cargo xtask build --verify`, beside `tests/confine.rs` (no libtest harness,
whose threads would refuse the box). They need Landlock ABI 6 and fail below
it, naming the kernel's ABI. `--verify` names each of `gro_cli`'s integration
tests with the features it runs under (`INTEGRATION_TESTS` in
`crates/xtask/src/snapshot.rs`), and an xtask unit test fails on a file of
`crates/gro_cli/tests/` no run names. `gro_sys`'s tests that close descriptors, clear
the environment, cap memory, or signal run in a child process, harness off.

**Vendored crates.** `vendor/` holds patched copies of published crates
(`deno_core`, patched for deterministic snapshots and what else its index
lists), wired through
`[patch.crates-io]` and outside the workspace and its checks (their own rustc
warnings print but fail nothing). [vendor/README.md](vendor/README.md) is the
index of every patch and the upgrade procedure, so a `deno_core` upgrade
re-vendors it per the README; edit vendored code only as a marked
`// gro patch:` ([vendor/CLAUDE.md](vendor/CLAUDE.md)). `check-rust` proves
each tree is its published crate plus its patch, and
`cargo xtask vendor-patch <name>` regenerates a patch. With the patch, a
clean checkout of one commit, the same toolchain, and the same cargo home
build the same `fgro`, byte for byte, wherever the checkout is. The
toolchain is the system's as well as Rust's: the C compiler and C library
reach the binary (its startup objects, the C a build script compiles), so on
CI it's the runner's image. `fgro` embeds the commit it was built from, so a
dirty tree (a changed file, or an untracked one git doesn't ignore: a
`-dirty` marker) or one git can't read as its own top level (no commit)
builds other bytes. `cargo xtask check-reproducible` holds it for the
release build: the release `fgro` built in the checkout and again in a
clone of its commit at another path, both with rustc wrappers turned off and
`PATH`'s `rustc`, compared byte for byte. It refuses a dirty tree, one
outside git, a variable cargo reads as a compile setting
(`CARGO_INCREMENTAL` and the like), a rustc wrapper in its own environment (the caller's, or one a
cargo config's `[env]` sets), or a cargo config only the clone's build
would read (in the clone's directory or above it, not above the checkout,
and not cargo home's). CI's `reproducible` job runs it on the release's
image, so it proves the release build path-independent there, but not a
release's own bytes, which build under another cargo home. It can't see
another machine, toolchain, or cargo home, a cargo config only the
checkout's build reads (above it, not above the clone), or what both builds
read alike on one machine: a directory's listing order on one filesystem,
the day's date, a cache they share.

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
  JS/TS under `crates/`, which `gro check` doesn't reach: `tsc` over every
  `tsconfig.json` under `crates/*/js/`, then the format check), then `gro check`.
  CI runs `check-rust` and `check-js` as its own jobs, so the steps live in
  the xtask only (`crates/xtask/src/check.rs`)
- `cargo xtask build [--release] [--verify]` - the snapshot `fgro`, built in
  two stages, at `<target>/gro_snapshot/<profile>/fgro`; `--verify` adds
  the snapshot remade by that `fgro` and compared byte for byte, clippy over
  the `snapshot` feature, `gro_runtime`'s tests of its runtimes built with
  the build's profile, the integration tests against that
  build, the size budgets, and a run with the build tree hidden
  (`unshare -rm`, which needs unprivileged user namespaces). A plain
  `cargo build` makes an `fgro` with no snapshot that runs only from its
  build tree and can't `eval`
- `cargo xtask check-reproducible [--dir <dir>]` - the release `fgro` built
  in a clean checkout and again in a clone of its commit under `<dir>`
  (default `$XDG_CACHE_HOME/gro/reproducible`), from the cleared
  environment the perf rule's base build uses, then compared byte for byte,
  a difference failing with the sections that differ and the clone kept;
  two release builds and the clone's target dir, so `check` leaves it to CI.
  `<dir>` is apart from the checkout, without `..`, and the check's own:
  missing, empty, marked `.check-reproducible` by an earlier run, or
  holding only the lock an earlier one left; it refuses any other rather
  than replace its `src`

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
- a workload project's `blake3` pins its committed files, and a run refuses
  files that hash otherwise; a change to them bumps the suite's `version` and
  re-pins, which a `gro_bench` test holds against `history.jsonl`
- a workload runs under every runtime, or the ones its `runtimes` names when
  it's of one runtime's own API: the baseline's `import_builtins`, a first
  import of a few builtin rows, runs under `fgro` alone, and an A/B runs it
  under the base copy too, so the baseline's A/B reports first imports,
  against `empty`
- `cargo xtask bench report` - each row's latest record against the previous
  and the last attested one
- Record a suite on a quiet machine, after `cargo xtask build --release` (so
  the bench's own build is a cache hit):
  `cargo xtask bench crates/gro_bench/suites/baseline/suite.json --attest-quiet`
  (or `suites/eval/suite.json`)

**The perf rule.** A change to anything the release `fgro` is built from
(`gro_runtime`, `gro_cli`, `gro_sys`, `gro_v8`, `vendor/`, the root
`Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, `rust-toolchain.toml`) runs
`cargo xtask bench crates/gro_bench/suites/baseline/suite.json --counters --base <merge-base>`
and the same for `crates/gro_bench/suites/eval/suite.json` before commit; a
test-only or doc-only change is outside it. The A/B runs alone, with nothing
else loading the machine. It builds the release `fgro` at that revision in a
clone outside the checkout, runs each `fgro` row beside the base copy
interleaved, prints instruction, cycle, and wall medians against the base's,
and records nothing. An instruction move past 2% needs a written reason in
review.

## Implementation reference

Core systems (src/lib/):

- CLI and task invocation: `gro.ts`, `invoke.ts`, `task_args.ts`, `task.ts`,
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
  plugin modules, the rendered `$app/env/*` shims) import `fs`, as they import the process
  modules, one per kind of authority: `env.ts`, `args.ts`, `exit.ts` (`exit`, `set_exit_code`),
  `stdin.ts`, `stdout.ts` (with `ProcessOutput`, the type `stderr.ts` shares), `stderr.ts`,
  `cwd.ts`, `pid.ts`, `runtime_version.ts`, the two streams' implementation in
  `internal/process_output.ts`; their errors grant-free in `env_error.ts` (`EnvError`,
  `is_env_error`) and `stdio_error.ts` (`StdioError`, `is_stdio_error`); library code takes a
  `cwd` string, a plain record, the CLI args, or a stream,
  but for `internal/fs_boundary.ts`, which reads `HOME` through `env` once at load for its protected directories;
  Gro's library and task code uses no `node:child_process` but `spawn.ts`, and
  no `node:fs` but the Node-only machinery
  [host_modules.md](src/docs/host_modules.md#where-to-import-them) lists;
  `src/test/host_module_declarations.test.ts`
  fails if a host module's emitted declarations name a type outside the ES libs
  and its short list of web types (`WEB_TYPES`), such as a Node or DOM-only type
- Errors: `error_detail.ts`, `format_error_detail` (a message and its chain of causes) and
  `format_error_causes` (the chain alone, after a stack), grant-free, how Gro and its users print
  a host module's error, whose `message` is only `kind: target`
- Spawn: `spawn.ts`, the spawn host module, `program`/`find`/`program_in`/`find_in`/`narrow`
  making pinned `SpawnProgram` handles with an explicit environment; `spawn_error.ts`, its
  `SpawnError` and `is_spawn_error`, grant-free ([docs](src/docs/host_modules.md#spawn))
- CLIs: `cli.ts` - `find_cli(spawn, name)` tries the project's `node_modules/.bin` and then
  `PATH`, resolved through a `Spawn`, so only roots import the spawn host module, as for `fs`
- Hashing: `blake3.ts`, the blake3 host module, BLAKE3 over bytes, `hash` and hashers from
  `create_hasher`, `create_keyed_hasher`, `create_derive_key_hasher`, over `@fuzdev/blake3-wasm`
  on Node and the native crate on `fgro`, grant-free ([docs](src/docs/host_modules.md#blake3))
- Network: `net.ts`, the net host module, `net.fetch` and `narrow` bound to hosts, following redirects itself so each `Location` is checked; `net_error.ts`, its `NetError` and `is_net_error`, grant-free ([docs](src/docs/host_modules.md#net))
- Watch: `watch.ts`, the watch host module, `watch.dir(handle, {recursive, signal, max_pending})` resolving, once the watch is in place, to a `Watcher`, an async iterable of `add`/`update`/`delete`/`rescan` events with paths relative to the handle, symlinks never followed, buffered to `max_pending`, over chokidar imported on first call and apart from the Filer's `internal/watch_dir.ts`; its errors are `FsError` ([docs](src/docs/host_modules.md#watch))
- Git: `git.ts`, library code over a `git` `SpawnProgram` a root builds with `to_git_program`;
  a read of the local repo captures with `out`, anything that may prompt uses `run` (or `out` with
  `stdin: 'inherit'` when its output is needed), a caller's
  branch or tag name passes `assert_git_ref_name` so it's never a refspec or pattern,
  and an origin goes after `--end-of-options`, while `git_pull` refuses one beginning with `-`
- Long-lived children: `spawn_restartable.ts`, restarts over one `SpawnProgram`
- Module handling: `modules.ts`, `format_file.ts`, `package_json.ts`, `task_args.ts`
- Env files: `env_files.ts`, the `.env` loader (`load_env`, `load_from_env`, `load_env_file`)
  that the rendered `$app/env/*` shims, `gro publish`, and `gro_plugin_server` read through;
  `env_declarations.ts`, the declarations' shapes, which imports no fs
- SvelteKit integration: `svelte_config.ts`, `sveltekit_shim_*.ts`
- Build tools: `build_cache.ts`, `build_cache_dependencies.ts`

Documentation: [Complete index at src/docs/README.md](src/docs/README.md) - Core
topics: [task](src/docs/task.md), [gen](src/docs/gen.md),
[plugin](src/docs/plugin.md), [config](src/docs/config.md),
[test](src/docs/test.md) | Workflows: [dev](src/docs/dev.md),
[build](src/docs/build.md), [deploy](src/docs/deploy.md),
[publish](src/docs/publish.md)
