# runtime

> the design of Gro's experimental Rust runtime;
> the summary, security model, and roadmap are in [the README](../../README.md#runtime)

Designed, not yet built, unless noted.

## Design

The runtime is a Rust binary embedding V8 through [rusty_v8](https://github.com/denoland/rusty_v8).
It's bootstrapped on [deno_core](https://crates.io/crates/deno_core)
and Deno's web-standards extension crates, used thinly behind gro's own seams
so each piece can be rewritten on its own as measurements justify it.

- **Web standards, not Node compat.** The API is the
  [WinterTC minimum common API](https://min-common-api.proposal.wintertc.org/)
  plus a small set of host modules
  ([`fuz_host`](https://github.com/fuzdev/fuz_host): fs, process, spawn, watch),
  each an explicit import.
  No `node:*`, no `npm:`, no CommonJS, no N-API, and no `process` global.
  The break is what makes the security model enforceable:
  Node's API hands every module ambient authority (`process`, `require`, and `node:fs`
  are reachable from anywhere), so a compat layer would bring it back.
  It also declines the years of work Bun and Deno each spent on Node compat;
  code that needs Node keeps running on Node.
- **Strict ESM resolution, no legacy algorithm.** Relative and absolute specifiers
  resolve as URLs with mandatory extensions: no probing, no directory index, no `main`.
  Bare specifiers resolve through a package's `exports` with a fixed condition set
  (`gro`, `import`, `default`) and `imports` for `#` subpaths, walking `node_modules`.
  A package without `exports`, or one that's CommonJS, fails loudly with the reason;
  the fix is a version with an ESM build, an ESM bundle of it vendored into the project,
  or running it on Node. Gro never pre-bundles a project's dependencies.
  Files on disk only: no `https:`, `jsr:`, or `npm:` imports.
- **No runtime namespace.** Beyond the web's globals, everything is an import.
- **TypeScript by type stripping only.** [tsv](https://github.com/fuzdev/tsv) strips types in-process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Erasable syntax only, like Node's stripper: no enums, namespaces, or parameter properties.
- **No ambient authority.** Permissions are a grant set passed to the runtime at construction,
  empty by default, not process-level flags checked at call time.
  Authority is lexically scoped: a module reaches only what it imports or is handed,
  the way a function reaches only the bindings in scope.
  Host modules are imports and the module loader is the boundary,
  so an ungranted graph fails at link time, not at the first call.
  The global object is the one dynamic channel: the web globals that reach the world
  (`fetch`, `WebSocket`) are declared, never inferred, and absent until then,
  and a net grant names hosts, never "all".
  Pure globals (`URL`, `TextEncoder`, streams, `console`) are always there;
  ops check the scoped cases (reads under a root).
  `-A` is the one explicit escape hatch, never a default.
  On Linux the grant set also becomes kernel rules for the whole invocation
  (Landlock for the filesystem and TCP, seccomp for the rest of the socket and
  process surface), installed before V8 starts a thread and inherited by every
  child process, so an op-check bug, an engine escape, or a spawned tool still lands in a box.
  The kernel layer is coarser than the runtime's own checks (paths and ports, not
  files carved out of a tree or hostnames), so it backstops them rather than replacing them.
- **Native code: first-party crates linked in, WASM as the portable path, FFI as a grant.**
  tsv and blake3 are crates in the binary, exposed as built-in modules with no boundary to cross.
  Third-party native code arrives as WASM, or as an embedder's own host modules.
  FFI (calling a native library from JS) is planned as a grant, denied by default,
  with the library loaded in its own confined process.
  There is no in-process FFI, so native code can't reach the runtime's memory or its grants.
  Calls cross a process boundary, so chatty native work belongs in WASM instead.
- **A toolchain, grown by need.** Run, eval, and tasks come first;
  then serving (TCP, TLS, WebSocket) so fuz_app's TS backends run on it,
  a test runner, bundled dev and build, and types emitted from the binary.
  tsv formats, linked in as crates. Nothing lands before a consumer needs it.
- **Embeddable.** `gro_runtime` is a library other Rust binaries embed,
  with a registration API for their own host modules; the `gro` binary is one embedder.
  The registration API is gro's own, not deno_core's,
  so embedders don't break as the core underneath is rewritten.
  Grants are a constructor argument, so an embedder is confined without flags.
  The crates publish when the binary replaces the npm package.
- **One global binary, pinned per project.** A project declares a compatible range under
  `engines.gro` in `package.json`, and the binary fails hard outside it.
  Svelte, Vite, and SvelteKit stay the project's own dependencies; the binary owns gro,
  never the project's compiler or framework.

## Compared to Deno

If you know Deno, the ingredients are familiar:
V8, web standards, TypeScript without a build step, and permissions.
Gro takes the other side of most of Deno's later choices:

- **Node:** Deno added `node:*`, `npm:`, and CommonJS;
  gro declines all three, since Node's API is ambient authority.
- **Imports:** Deno resolves `https:`, `jsr:`, and `npm:` specifiers;
  gro reads files on disk, resolved strictly.
- **Namespace:** Deno has `Deno.*`; gro has no runtime namespace.
- **Permissions:** Deno's are process-level flags checked at call time;
  gro's are a grant set per module graph, inferred from imports,
  authorized by a committed policy file, and backed by kernel rules.
- **TypeScript:** Deno transpiles the full language;
  gro strips erasable syntax only, so positions never move.

## What the hermetic design opens up

- **Grants that read like the web.** `typeof fetch` stays the feature check the web already uses,
  and the error for an undeclared one names the grant to add.
  `gro run` and `gro eval` get exactly their file's graph; `gro task` gets the union of
  what the invocation reaches, since a task holds the context (config, `invoke_task`).
- **Run classes.** Every run is hermetic (no network or spawn, clock and randomness
  fixed or denied, every read recorded), confined (bounded but not replayable),
  or unconfined (`-A`, `publish`, `deploy`).
  The class shows in the policy file and the task listing,
  so a gen file that stops being hermetic is a reviewed diff.
- **Deterministic runs.** Clock and randomness are capabilities (ambient, denied, or fixed),
  and hermetic runs pin the time zone and locale,
  so an eval or gen run replays exactly.
  Eval and gen see the project at a fixed root, never this machine's absolute paths,
  so their output is the same on every machine.
- **Memoized gen.** A gen file declares the inputs it may read,
  and its output is keyed by the runtime, its module graph, and the files it actually read,
  so `gro gen --check` becomes a cache probe.
- **One module graph.** The loader's graph is the only graph:
  it keys caches and gen invalidation, drives watch,
  and lists tasks by parsing instead of executing them.
- **Cheap isolation.** V8 isolates and contexts are cheap in one process,
  so gen files can run in parallel, each confined, with no process spawn;
  trust tiers (project, ecosystem, third party) get their own global object.
- **Startup.** A snapshot of the host, a code cache keyed by content,
  and lazy host init.
- **Fewer platform-isms.** URL resolution with no path separators and argv-only spawn
  (never a shell string) keep platform differences in a handful of files.
