# runtime

> Gro's experimental JS runtime: a Rust binary on V8 with granular control over what code can reach;
> why it exists, the security model, the design, and the roadmap

> **status**: experimental, direction settled, design in progress -
> `crates/` holds the Rust workspace, and `fgro run <file>` runs a JS or TypeScript module
> from disk on V8 with the web-standard globals, from a startup snapshot when built by
> `cargo xtask build`, resolving packages in `node_modules` by their `"exports"`
> (x86_64 Linux only for now).
> It takes only the entry file, with no script arguments yet;
> the README's [Develop](../../README.md#develop) section says how to build and run it.
> Everything below is designed; only what the [roadmap](#roadmap) marks done is built.
> It doesn't have every answer yet (the [open questions](#open-questions) say which),
> and feedback is welcome in [discussions](https://github.com/fuzdev/gro/discussions).

Gro is becoming its own JS runtime: a Rust binary that embeds V8,
built on web standards instead of Node compat,
with granular control over what code can reach.
By default a run is confined: a module reaches only what it imports or is handed,
the way a function reaches only the bindings in scope,
so what a run can reach is a static fact about its import graph,
reviewed like a lockfile, and the common case has nothing to declare.
Runs can be hermetic too: a hermetic run fixes or denies clock and randomness,
so what it depends on is known, and it replays and caches by construction.
It's built for code you didn't write: your dependencies and your agents.

Four terms carry the rest of this document.
_Host modules_ are the imports that reach the OS (fs, process, spawn, watch, net),
each a visible edge in the graph.
_Authority globals_ are the web globals that reach the world (`fetch`, `WebSocket`),
the one authority that isn't declared by import, since a global reaches every module in the graph.
The _policy file_ (currently `gro.policy.json`) is the committed record of what was accepted in review.
And every run is one of three _kinds_ by what it reaches and depends on:
hermetic (replayable), confined (bounded), or unconfined.

Its primary verb is `gro run <file>`: one module graph, no ambient authority,
nothing reachable that the graph didn't import or the policy file didn't accept.
`gro eval` is its confined one-shot sibling.
`gro task <path>` is the workflow layer on top: a project task runs by its full path,
never by a searched name, while the builtins stay first-class subcommands (`gro check`, `gro dev`).

It starts by running gro's own TS surface (tasks, gen, config)
and grows into the toolchain for the whole Fuz stack,
including the server side of [fuz_app](https://github.com/fuzdev/fuz_app),
which keeps supporting Node and Bun beside it.
Until the binary replaces the npm package, the TypeScript version keeps working on Node
and the binary is named `fgro`.

## Why

The runtime started as an exit.
The Fuz stack ran on two runtimes it doesn't own:
Node for gro and the tools it wraps like SvelteKit and Vite,
and Deno for scripts, tests, and evaluating config it didn't trust under permission flags.
When [Cloudflare bought Deno](https://deno.com/blog/cloudflare) and put the runtime on bug and security fixes
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
One runtime we own in place of two we don't, and one embeddable host for the toolchain.
The sandbox ships first, as `gro eval`, long before gro runs on itself.

It isn't a Deno replacement for everyone.
Code that needs Node keeps running on Node,
and the Node tools the stack still uses run as subprocesses until each is replaced.
It isn't only a JS sandbox either: V8 runs WASM, and the same grants confine
any language compiled to it (see [polyglot sandboxing](#what-the-hermetic-design-opens-up)).

## Secure by default

Code starts with nothing and gets only what it's granted.
There's no flag to remember and no mode to opt into:
the default is the confined case, and widening it is a visible act, reviewed like a lockfile change.
Designed, not yet built, a run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared build roots,
  and the public env names it declares
- **only after it's accepted in review:** the network, by named hosts
  (a blanket grant exists, accepted the same way and shown as the widest class),
  spawning processes, reads outside the project, private env names,
  and any authority reached through a dependency
- **never short of `-A`:** `.env*` files except through the env module,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`
- **for `gro eval`:** its own module graph, read-only, with no network, writes, env, or spawn,
  and the JIT off unless a flag turns it on
- **for native code:** never the runtime's process, not even under `-A`;
  FFI is a grant that loads the library in its own confined process

A script's needs are a static property of its import graph,
so least privilege costs nothing to write:
a file that imports only `fuz_util` and the fs module can't spawn or fetch.
That is `gro run`'s unit, one file's graph; `gro task` runs under the union of what the invocation reaches,
and a Node tool it spawns is confined by the kernel box, not the grant set.
The source proposes (inferred host modules, plus a static `grants` literal naming hosts
and the like) and the committed policy file authorizes:
a dependency bump that newly reaches the filesystem, spawn, or the network fails
`gro check` until the diff is accepted, the same shape as a lockfile change.

The policy file is also the guardrail for agents:
what an agent's code may reach sits in one small file a person can audit,
the runtime enforces it, and a widening is a diff in review.
How to keep an agent from editing that file itself is open:
prompting and harness rules cover the normal case and aren't a structural guarantee,
and a signed acceptance is designed for when it's needed
([open questions](#open-questions)).

Every run is one of three kinds, derived from its grants,
recorded in the policy file, and shown in the task listing,
so a gen file that stops being hermetic is a reviewed diff:

| kind       | network and spawn                      | clock and randomness | reads                         | replays |
| ---------- | -------------------------------------- | -------------------- | ----------------------------- | ------- |
| hermetic   | none                                   | fixed or denied      | under the root, each recorded | exactly |
| confined   | by grant                               | real                 | by grant                      | no      |
| unconfined | everything (`-A`, `publish`, `deploy`) | real                 | everything                    | no      |

Gen files and `gro eval` are hermetic by default.

A grant is always a grant of something specific, so granting individually,
even a blanket network grant, keeps a run confined, just with a wider reach shown in the diff.
That includes the authority globals: `fetch` reaches every module in the graph,
but only to the hosts accepted.
Only `-A` (`--allow-all`, as in Deno) runs unconfined, and a grant as strong as `-A`,
like spawning a shell, is flagged that way in the diff.

The guarantees come in two kinds, and it's worth knowing which is which.
By construction, with no setting that changes them:
authority enters by import, so what a graph can reach is a static fact;
there's no `process` global, no Node compat, no in-process FFI, no remote import,
and no runtime namespace to reach around the loader;
and a WASM module's imports are its capability declaration the same way.
By default, where a policy entry or `-A` widens them and the diff shows it:
the read and write roots, the net hosts, the authority globals, the deny overlays,
the empty environment for children, the kernel rules, and whether a run stays hermetic.
A widening of the second kind is a diff accepted in review; the first kind has nothing to widen.

The exceptions are explicit and labeled.
`-A` is the one escape hatch, never a default.
`publish` and `deploy` push and use credentials, so they're named unconfined phases,
shown that way in the policy diff, with credentials passed to the one process that needs them.
The kernel backstop is Linux-only until the other platforms get an equivalent.

Security-first runtimes earned a reputation for friction:
flags on every run, prompts mid-execution, and `-A` as the habit that follows.
Gro has none of those: a normal run has no flags and nothing to declare,
and the one moment worth a human's attention, the graph newly reaching the network,
a process, or files outside the project, arrives as a diff with a copyable fix, never a prompt.
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
Until the Node tools leave the loop, an invocation that spawns them
is as confined as the kernel box, paths and ports, not the grant set.
Outside the model: the unconfined phases (`publish`, `deploy`, `-A`),
and on macOS and Windows, anything past the runtime's own checks
until those platforms get a kernel equivalent.

## Design

The runtime prioritizes, in order:

1. security: no ambient authority, confinement by construction, a kernel backstop under it,
   measured by a gate suite of escape probes the way speed is measured by the bench
2. web-standards conformance: the WinterTC minimum common API,
   graded by its WPT subset under a full grant set
3. developer control and ergonomics: no prompts, nothing to declare in the common case,
   and an escape hatch beside every default (`-A`, the globals grant, the JIT flag for eval),
   since a boundary that's painful gets bypassed
4. speed: startup first, since short-lived invocations dominate an agent-driven loop;
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
5. binary size and memory

Staying simple and small is overarching, and the security story depends on it:
a surface small enough to audit, where every path to the OS is an import or a declared global.
Gro will run the same V8 as Node and Deno, so steady-state compute will be at parity;
the wins are everything around the engine (startup, loading, caching, work avoided).
Gro isn't security maximalism: the defaults are safe and the user decides when to widen them.

The runtime is a Rust binary embedding V8 through [rusty_v8](https://github.com/denoland/rusty_v8).
It's bootstrapped on [deno_core](https://crates.io/crates/deno_core)
and Deno's web-standards extension crates, used thinly behind gro's own seams
so each piece can be rewritten on its own as measurements justify it.

- **Web standards, not Node compat.** The API is the
  [WinterTC minimum common API](https://min-common-api.proposal.wintertc.org/)
  plus a small set of host modules
  ([`fuz_host`](https://github.com/fuzdev/fuz_host): fs, process, spawn, watch, net),
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
  A package without `exports`, or one that's CommonJS, fails loudly with the reason and the fix.
  Files on disk only: no `https:`, `jsr:`, or `npm:` imports,
  and gro never pre-bundles a project's dependencies.
  Otherwise it follows Node's algorithm, departing where Node guesses, reads past the project,
  or substitutes a pattern unchecked; [resolver](resolver.md) has the full list.
- **No runtime namespace.** Beyond the web's globals, everything is an import.
- **TypeScript by type stripping only.** [tsv](https://github.com/fuzdev/tsv) strips types in-process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Erasable syntax only, like Node's stripper: anything with runtime semantics
  (an `enum`, a parameter property) is refused with its position and a fix, never transformed.
  [typescript](typescript.md) has the rules, the refused list, and the file types.
- **No ambient authority.** The grant set is a constructor argument of the runtime,
  empty by default, not process-level flags checked at call time.
  Host modules are imports and the module loader is the boundary,
  so an ungranted graph fails at link time, not at the first call.
  The network is an import too: the host's net module exports a `net` handle
  (`net.fetch`, bound to the accepted hosts) that's narrowed and passed down like any other capability,
  so authority is visible at every use site, nothing shadows the web's `fetch`,
  and a net grant is inferred from the graph like a filesystem one, naming hosts;
  a blanket one is never proposed for you, only accepted, and the policy diff shows it as the widest class.
  The authority globals are a separate, wider grant,
  since a global reaches every module in the graph: declared, never inferred, and absent until then.
  The global object is the one dynamic channel, and it's sealed after bootstrap:
  a package that writes to it needs a policy entry of its own,
  and a handed capability can't be stashed there as a new global.
  (Existing globals and the shared prototypes stay mutable, as in browsers.
  A reference parked on one never widens what its holder could reach,
  but a module that patches a shared prototype can interpose on calls that pass through it,
  so against a dependency treated as hostile the realm isn't the boundary;
  freezing the intrinsics is a per-tier hardening, not the default.)
  Pure globals (`URL`, `TextEncoder`, streams, `console`) are always there;
  ops check the scoped cases (reads under a root).
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
  Calls cross a process boundary, so chatty native work belongs in WASM instead,
  where a module reaches only the imports the host hands it
  (see [polyglot sandboxing](#what-the-hermetic-design-opens-up)).
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
- **Startup.** A snapshot of the host, a code cache keyed by content, and lazy host init.
- **Fewer platform-isms.** URL resolution with no path separators and argv-only spawn
  (never a shell string) keep platform differences in a handful of files.

## Compared to Deno, Node, and Bun

[comparison](comparison.md) has the full account:
the design differences from Deno in prose
(the ingredients are the same; the structural difference is that authority enters by import,
with no namespace to reach around the loader),
notes on Node and Bun,
and the same ground as tables by axis, with the bench's measured startup and size.
In one line: Deno's lockfile pins which code runs;
gro's policy file also pins what that code can reach,
and treats the second lock like the first: committed, diffed, reviewed.

## What the hermetic design opens up

- **Grants that read like the web.** `typeof fetch` stays the feature check the web already uses,
  and the error for an undeclared one names the fix: the import first, the global grant second.
  `gro run` and `gro eval` get exactly their file's graph; `gro task` gets the union of
  what the invocation reaches, since a task holds the context (config, `invoke_task`).
- **Deterministic runs.** Clock and randomness stay the web's globals,
  set per run kind (real, denied, or fixed) rather than imported,
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
  trust tiers (project, ecosystem, third party) could get their own global object
  (per-tier contexts, on the roadmap's maybe list).
- **Polyglot sandboxing.** V8 runs WASM, and a WASM module imported as an ES module
  declares its capabilities by its imports, the same thing a JS module's imports are
  (one instantiated from bytes at run time is a declared entry, like a computed import).
  So one policy file, one inference, and one kernel backstop cover every language that targets WASM,
  and a tool, a plugin, or an agent's script written in Rust, C, Go, Zig, or Python
  runs under the grants its imports declare, beside the TS that calls it, in one process.
  The WASI shape maps onto it directly: the grant set is the host side of the module's world,
  and read roots are its preopens.
  Code that must be treated as hostile goes here or in its own process, never under a looser grant,
  which is what lets the runtime's scope grow past JS without widening the boundary.

## Open questions

Security is the north star, and neither it nor speed is finished.
The design doesn't have every answer; these are the ones it knows it's missing:

- **Keeping an agent out of the policy file.** Review, harness rules, and a push gate cover the
  normal case; a signed acceptance that needs a person's presence is designed, built only if
  the measured widening rate makes its friction worth it.
- **Replay needs deterministic scheduling, for gen.** `gro eval` only loads modules, which ESM orders,
  but a hermetic gen run's recorded reads complete on the event loop in no fixed order,
  so it can resolve promises differently between runs.
  The host has to order completions for "replays exactly" to hold by construction,
  and the cost is unmeasured.
- **Interposition through shared prototypes.** A same-realm dependency can patch an intrinsic
  and intercept a holder's calls. Freezing the intrinsics by default, per-tier contexts,
  or stating the limit (the current answer) is undecided; the gate suite carries a probe either way.
- **Where the builtins' grants live.** `gro check` spawns Node tools through gro's own tasks,
  so "nothing to declare" holds only once the builtins' grants ship with the binary
  and a project's policy file holds project tasks alone.
- **Eval and WASM.** With the JIT off, WASM support is limited,
  so a config that imports a WASM module may need the JIT flag.

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
- [ ] the first release: `fgro` 0.1.0 for Linux x86_64 as a GitHub Release asset,
      `run` and `eval`, with checksums and a provenance attestation
- [ ] the security gate: a suite of escape probes run under every grant set,
      with every web global and member classified by the authority it carries as the oracle,
      grown from `gro eval`'s acceptance tests and passed by every milestone after,
      with the type strip fuzzed over real packages, since it runs on `node_modules`
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
