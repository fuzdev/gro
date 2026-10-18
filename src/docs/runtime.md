# runtime

> Gro's experimental JS runtime: a Rust binary on V8 with granular control over what code can reach;
> the roadmap, why it exists, the security model, and the design

> **status**: experimental, direction settled, design in progress -
> `crates/` holds the Rust workspace, and `fgro run <file>` runs a JS or TypeScript module
> from disk on V8 with the web-standard globals, from a startup snapshot when built by
> `cargo xtask build`, resolving packages in `node_modules` by their `"exports"`
> (x86_64 Linux only for now).
> It takes only the entry file, with no script arguments yet;
> the README's [Develop](../../README.md#develop) section says how to build and run it.
> Everything below is designed; only what the roadmap marks done is built.

Gro is becoming its own JS runtime: a Rust binary that embeds V8,
built on web standards instead of Node compat,
with granular control over what code can reach.
It's confined by construction: a module reaches only what it imports or is handed,
the way a function reaches only the bindings in scope,
so what a run can reach is a static fact about its import graph,
reviewed like a lockfile, and the common case has nothing to declare.
Runs can be hermetic too: clock and randomness are capabilities,
so what a run depends on is known, and it replays and caches by construction.
It's built for code you didn't write: your dependencies and your agents.

Its primary verb is `gro run <file>`: one module graph, no ambient authority,
nothing reachable that the graph didn't import or the
[policy file](#secure-by-default) didn't accept.
`gro eval` is its confined one-shot sibling.
`gro task <path>` is the workflow layer on top: a project task runs by its full path,
never by a searched name, while the builtins stay first-class subcommands (`gro check`, `gro dev`).

It starts by running gro's own TS surface (tasks, gen, config)
and grows into the toolchain for the whole Fuz stack,
including the server side of [fuz_app](https://github.com/fuzdev/fuz_app),
which keeps supporting Node and Bun beside it.
Until the binary replaces the npm package, the TypeScript version keeps working on Node
and the binary is named `fgro`.

## Roadmap

The first target is retiring Deno from the Fuz stack before its end of life
in October 2027 (see [Why](#why)).
Until Node leaves much later, third-party Node tools (Vite, vitest, eslint, svelte-check)
run as Node subprocesses under the invocation's kernel rules.

- [x] Rust workspace and a benchmark harness that times gro beside Node, Deno, and Bun
      on the same workloads, the baseline every milestone is gated against
- [x] `gro run` (as `fgro run` for now): ESM from disk on V8, web-standard globals, a startup snapshot
- [x] TypeScript by tsv's type strip, in process
- [x] strict package resolution: `node_modules` by `"exports"`, and `"type": "module"`
      (`#` imports come later)
- [ ] `gro eval`: confined, deterministic one-shot eval of a TS module to JSON
      under the kernel backstop, for any tool that evaluates config;
      it replaces Deno for config eval in the Fuz stack
- [ ] the security gate: a suite of escape probes run under every grant set,
      with every web global and member classified by the authority it carries as the oracle,
      grown from `gro eval`'s acceptance tests and passed by every milestone after
- [ ] on the Node gro: `gro task <path>` replaces the bare `gro <name>` task search
      and the loader turns strict, so the scripts the Deno exit ports are written once, in the final shape;
      then `gro check` infers each task's grants
      and diffs them against the committed policy file, reporting before anything enforces it
- [ ] the Deno exit: config eval on `gro eval`, the stack's other Deno uses on Node
- [ ] startup: a tuned snapshot, a content-keyed code cache, lazy host init
- [ ] gro on gro: tasks, gen, and check run on the runtime, enforcing the policy file
      (until then the CLI passes every host module; the network stays declared)
- [ ] the Svelte compiler in-engine
- [ ] macOS and Windows builds
- [ ] binary-only: `fgro` becomes `gro`, the npm package carries types, and the crates publish
- [ ] bundled dev and build on the same host, and Vite leaves the loop
- [ ] serving, a test runner, and types from the binary:
      fuz_app's server side runs on gro, and Node leaves the stack's toolchain

Future (unknown order):

- FFI as a grant, in its own confined process
- deterministic tasks and gen: clock and randomness fixed or seeded,
  gen as a memoized function of its graph (`gro eval` denies both from the start)
- owning the core: deno_core's pieces replaced one at a time, measured
- `fetch`, `crypto.subtle`, and `WebSocket` as consumers need them
- size: filtered ICU data, V8 build flags

Maybe later or never:

- per-trust-tier V8 contexts
- a custom V8 build

## Why

The runtime started as an exit.
The Fuz stack ran on two runtimes it doesn't own:
Node for gro and the tools it wraps like SvelteKit and Vite,
and Deno for scripts, tests, and evaluating config it didn't trust under permission flags.
When Cloudflare bought Deno and put the runtime on bug and security fixes
until its end of life in October 2027, everything on it had to move.
The scripts and tests go to Node.
The sandbox was the part worth owning:
a security boundary is the last thing to leave on an unmaintained runtime,
and the first thing to want by construction instead of by flag.

That forced the question of what runtime the stack wants, and the answer was smaller than either.
Gro already runs the stack's TypeScript in-process: tasks, gen files, and config, through its Node loader.
The runtime is that loader becoming the host, on the pieces that outlive the sale
([rusty_v8](https://github.com/denoland/rusty_v8), which Cloudflare keeps maintaining,
and [deno_core](https://crates.io/crates/deno_core) as a bootstrap, used thinly behind gro's own seams
so each piece can be rewritten as measurements justify it).
One runtime we own in place of two we don't,
confinement and replay for code you didn't write,
and one embeddable host for the toolchain.
The sandbox ships first, as `gro eval`, long before gro runs on itself.

It isn't a Deno replacement for everyone.
Code that needs Node keeps running on Node,
and the Node tools the stack still uses run as subprocesses until each is replaced.
It isn't only a JS sandbox either:
V8 runs WASM, and a WASM module reaches only the imports the host hands it,
so the same grants confine Rust, Go, or Python compiled to WASM beside the TS that calls them.
One boundary for every language that targets it is what lets the scope grow
(see [polyglot sandboxing](#what-the-hermetic-design-opens-up)).

The runtime prioritizes, in order:

1. security: no ambient authority, confinement by construction, a kernel backstop under it,
   measured by a gate suite of escape probes the way speed is measured by the bench
2. web-standards conformance: the WinterTC minimum common API, graded by its WPT subset
3. speed: startup first, since short-lived invocations dominate an agent-driven loop;
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
4. binary size and memory
5. embeddability: published crates with a registration API, the binary being one embedder

Staying simple and small is overarching, and the security story depends on it:
a surface small enough to audit, where every path to the OS is an import or a declared global.
Gro will run the same V8 as Node and Deno, so steady-state compute will be at parity;
the wins are everything around the engine (startup, loading, caching, work avoided).
Feedback is welcome to help navigate these tradeoffs.

## Secure by default

Code starts with nothing and gets only what it's granted.
There's no flag to remember and no mode to opt into:
the default is the confined case, and widening it is a visible, human act.
Designed, not yet built, a run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared build roots,
  and the env names it declares
- **only after a human accepts it:** the network, by named hosts
  (a blanket grant exists, accepted the same way and shown as the widest class),
  spawning processes, reads outside the project,
  and any authority reached through a dependency
- **never short of `-A`:** `.env*` files except through the env module,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`
- **for `gro eval`:** its own module graph, read-only, with no network, writes, env, or spawn,
  and the JIT off
- **for native code:** never the runtime's process, not even under `-A`;
  FFI is a grant that loads the library in its own confined process

A script's needs are a static property of its import graph,
so least privilege costs nothing to write:
a file that imports only `fuz_util` and the fs module can't spawn or fetch.
The source proposes (inferred host modules, plus a static `grants` literal naming hosts
and the like) and a committed policy file authorizes:
a dependency bump that newly reaches the filesystem, spawn, or the network fails
`gro check` until a human accepts the diff, the same shape as a lockfile change.

The exceptions are explicit and labeled.
`-A` is the one escape hatch, never a default.
`publish` and `deploy` push and use credentials, so they're named unconfined phases,
shown that way in the policy diff, with credentials passed to the one process that needs them.
The kernel backstop is Linux-only until the other platforms get an equivalent.

Security-first runtimes earned a reputation for friction:
flags on every run, prompts mid-execution, and `-A` as the habit that follows.
Gro's grants come from the import graph,
so a normal run has no flags and nothing to declare,
and the policy file changes only when the graph newly reaches the network,
a process, or files outside the project.
That is the one moment worth a human's attention,
and it arrives as a diff with a copyable fix, never a prompt.
The costs are elsewhere, and stated plainly:
no Node compat, strict ESM, and the authority globals absent until declared.

## Threat model

Grants contain overreach:
a dependency, a task, or an agent that reaches for authority nobody accepted
fails before it runs (an unaccepted host module fails at link time,
an undeclared global isn't there), and the widening shows up as a diff in review.
V8 itself isn't treated as the boundary against code built to escape it.
An engine exploit lands in the kernel backstop,
which applies the same grants to the whole invocation and every child process,
and `gro eval` and FFI each run in a process of their own.
Code that must be treated as hostile belongs in WASM or a separate process,
not under a looser grant.
Outside the model: the unconfined phases (`publish`, `deploy`, `-A`),
and on macOS and Windows, anything past the runtime's own checks
until those platforms get a kernel equivalent.

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
  (`gro`, `import`, `default`, matched in the package's own key order),
  and `imports` for `#` subpaths (not built yet), walking `node_modules`.
  A package without `exports`, or one that's CommonJS, fails loudly with the reason;
  the fix is a version with an ESM build, an ESM bundle of it vendored into the project,
  or running it on Node. Gro never pre-bundles a project's dependencies.
  Files on disk only: no `https:`, `jsr:`, or `npm:` imports.
  Otherwise it follows Node's algorithm, with these deliberate departures:
  the `node_modules` walk ends at the project root, the first `package.json` outside `node_modules`
  that isn't a type marker (a JSON object holding only `"type"` and `"sideEffects"`,
  like `dist/esm/package.json`), so any other one ends it, named or not, failing closed:
  an uninstalled project fails instead of resolving a parent directory's packages,
  and npm and yarn workspaces, whose packages install at the workspace's root, aren't supported;
  every `package.json` the walk passes outside `node_modules` is read, to tell a marker from a bound,
  so one that isn't valid (not JSON, a `"name"` or `"type"` that isn't a string, or a special file)
  fails the import, where Node never reads it;
  a duplicated key keeps its last value, as `JSON.parse` does, where Node checks every occurrence;
  a `.js` file with no `package.json` above it is refused, where Node guesses its format from the code
  (`.mjs` needs none);
  `.ts` inside `node_modules` is stripped like any other, where Node refuses;
  no other condition matches (`node`, `require`, `browser`, `svelte` until the Svelte compiler is in);
  and a pattern's match can't hide a `..` behind a tab or a line break (`.\t.`), which URL parsing drops,
  where Node substitutes it unchecked and loads what it names outside the package.
- **No runtime namespace.** Beyond the web's globals, everything is an import.
- **TypeScript by type stripping only.** [tsv](https://github.com/fuzdev/tsv) strips types in-process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Erasable syntax only, like Node's stripper: no enums, value namespaces, parameter properties,
  `import =` or `export =`, angle-bracket assertions, or an unparenthesized `as` whose removal
  would regroup the expression (`-x as number ** 2`); each is refused with its position
  and a fix, never transformed. Imports follow `verbatimModuleSyntax`: `import {type T}` keeps
  the import, so its side effects run, while `import type` and `export type … from` drop entirely.
  `.ts` and `.mts` load as ES modules; `.cts` (CommonJS), `.tsx` (JSX), declaration files,
  and Svelte modules (`.svelte.ts`, `.svelte.test.ts`, any name with `.svelte.` ending in
  `.js` or `.ts`, which need the Svelte compiler) are refused with the reason.
- **No ambient authority.** Permissions are a grant set passed to the runtime at construction,
  empty by default, not process-level flags checked at call time.
  Authority is lexically scoped: a module reaches only what it imports or is handed,
  the way a function reaches only the bindings in scope.
  Host modules are imports and the module loader is the boundary,
  so an ungranted graph fails at link time, not at the first call.
  The network is an import too: the host's net module exports a `net` handle
  (`net.fetch`, bound to the accepted hosts) that's narrowed and passed down like any other capability,
  so authority is visible at every use site and nothing shadows the web's `fetch`;
  so a net grant is inferred from the graph like a filesystem one, and names hosts;
  a blanket one is never proposed for you,
  only accepted, and the policy diff shows it as the widest class.
  The global object is the one dynamic channel, and it's sealed after bootstrap.
  The web globals that reach the world (`fetch`, `WebSocket`) are a separate, wider grant,
  since a global reaches every module in the graph: declared, never inferred, and absent until then.
  A package that writes to the global object needs a policy entry of its own,
  and a handed capability can't be stashed there to become ambient again.
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
  WASM is the polyglot half of the sandbox: a WASM module has no ambient authority by construction,
  it reaches only the imports the host hands it,
  so the same grant set confines Rust, C, Go, Zig, or Python compiled to WASM
  exactly as it confines a TS module, with the module's imports as its capability declaration either way.
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
V8, web standards, TypeScript without a build step, and permissions off by default.
Gro starts there and takes the other side of most of Deno's later choices:

- **Node:** Deno added `node:*`, `npm:`, and CommonJS;
  gro declines all three, since Node's API is ambient authority.
- **Imports:** Deno resolves `https:`, `jsr:`, and `npm:` specifiers, fetching code at run time
  (a static remote import needs no permission);
  gro reads files on disk, resolved strictly, so the graph is whatever the lockfile installed.
- **Namespace:** Deno has `Deno.*`; gro has no runtime namespace.
- **TypeScript:** Deno transpiles the full language;
  gro strips erasable syntax only, so positions never move.
- **Permissions:** Deno's are process-level flags checked at call time;
  gro's are a grant set per module graph, inferred from imports,
  authorized by a committed policy file, and backed by kernel rules.

The sandbox is where the distance is largest.
Deno's lockfile pins which code runs; gro's policy file also pins what that code can reach,
and treats the second lock like the first: committed, diffed, reviewed.

- **Day to day.** A Deno run carries its flags on every invocation
  (`--allow-net=…`, `--allow-read=…`), bakes them into a task, or takes `-A`.
  A gro run carries nothing: the grants come from the import graph,
  and the policy file changes only when the graph newly reaches authority,
  which is lockfile cadence, not run cadence.
- **Who holds a grant.** A Deno permission belongs to the thread, so every module on it holds it.
  A gro grant belongs to the module graph that imports the host module,
  and a dependency's reach is attributed to the import chain that brought it in,
  so the diff says which package newly needs the network, not only that something does.
  A capability handed down by reference stays attributed to the module that imported it;
  handing never widens.
- **Where it fails.** Deno denies at the call, deep inside a dependency,
  with a `NotCapable` error the code can catch and swallow.
  Gro denies before the code runs: an ungranted host module doesn't link,
  an undeclared `fetch` isn't there.
  The host reports every denial itself, so a `try/catch` can't hide one,
  and under `gro check` and CI any denial fails the run.
- **How it widens.** Deno prompts when stdout is a terminal, takes a flag,
  or defers every check to an external broker process.
  Gro never prompts and has no dynamic decision authority:
  the source proposes, a human accepts the diff, the pending run says so,
  and no hint ever suggests `-A`.
- **Children.** Deno's `--allow-run` is unconfined; a spawned process has everything.
  Gro's grant set becomes kernel rules for the invocation and every child,
  so spawned tools run in the same box, from an argv (never a shell string),
  with executables resolved when granted and an environment that is empty plus an allowlist.
- **Native code.** Deno's FFI runs in-process under `--allow-ffi`;
  gro has no in-process FFI at all, and the library gets its own confined process.
- **What's denied regardless.** Inside a granted read or write root,
  `.env*` files, credential files, and writes under `.git` and `node_modules` stay denied short of `-A`,
  and credentials reach the one process that needs them as named grants, never as file reads.
- **Determinism.** Clock and randomness are capabilities (ambient, denied, or fixed),
  so a run can be hermetic and replay exactly; Deno has no such mode.
- **Eval.** `gro eval` runs in its own process with the JIT off, no code generation from strings,
  and a heap cap, and the binary reads no environment variables in confined mode,
  so nothing outside the flags changes what a run may do.

Deno has pieces gro doesn't: workers with a reduced permission set,
a runtime API to query, request, and revoke permissions, `--deny-*` overrides,
the permission broker, and years of production hardening.
Gro's claim is the shape of the boundary, not its maturity.

## What the hermetic design opens up

- **Grants that read like the web.** `typeof fetch` stays the feature check the web already uses,
  and the error for an undeclared one names the fix: the import first, the global grant second.
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
- **Polyglot sandboxing.** V8 runs WASM, and a WASM module's imports are its capability declaration,
  the same thing a JS module's imports are.
  So one policy file, one inference, and one kernel backstop cover every language that targets WASM,
  and a tool, a plugin, or an agent's script written in Rust or Python
  runs under the grants its imports declare, beside the TS that calls it, in one process.
  The WASI shape maps onto it directly: the grant set is the host side of the module's world,
  and read roots are its preopens.
  Code that must be treated as hostile goes here or in its own process, never under a looser grant,
  which is what lets the runtime's scope grow past JS without widening the boundary.
- **Fewer platform-isms.** URL resolution with no path separators and argv-only spawn
  (never a shell string) keep platform differences in a handful of files.
