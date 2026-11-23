# runtime

> Gro's experimental JS runtime: a Rust binary on V8 with granular control over what code can reach;
> why it exists, the model in a page, the design, and the roadmap

> **status**: experimental, with the direction settled and the design in progress.
> Built today, for x86_64 Linux alone (its crates refuse to compile for another OS), as `fgro` (renamed `gro` when ready):
>
> - `fgro run <file>` runs a JS or TypeScript module from disk on V8, using Deno's libraries,
>   with strict resolution and origin containment ([run](run.md))
> - `fgro eval <module>` evaluates one to JSON data, jitless, with the clock and randomness denied,
>   its heap, memory, and time limited, and its process in a kernel box read-only to its project
>   ([eval](eval.md))
> - `fgro <command>` runs a command of Gro's, like `fgro check`, from Gro's own code embedded in the binary,
>   its grants read natively from the command's closure, in the run's kernel box with every tool it starts,
>   or runs the project's Node Gro and says why
>   ([Gro's workflow](#gros-workflow-on-the-runtime))
>
> Designed, not built: the host modules not marked built in [host modules](host_modules.md#the-modules) (`net`, `watch`, and the standard streams among them;
> every host module exists as its Node implementation), the grant flags beyond those listed in [run](run.md#usage), the policy file,
> and the seal on the global object.
> So are the box for `fgro run` and a box for each tool inside a command's ([box](box.md)).
> Until they land, `fgro run` gives code no API past its own module graph, the console, and the builtin modules [host modules](host_modules.md#the-modules) marks built,
> but the process itself isn't confined or limited: against code that exploits V8 it's an ordinary process with your user's authority,
> the same class as Deno with no permissions granted;
> and with no exploit at all, a run that `--allow-run=` or `--allow-run-in=` grants a program is close to `-A`, as Deno's `--allow-run` is.
> A command of Gro's runs in the run's box, so a tool its closure grants is bounded by that box's rules
> ([box](box.md#what-a-commands-box-admits)), not by the grants.
> The [roadmap](#roadmap) marks what's built, the design doesn't have every answer yet
> ([open questions](#open-questions)), and feedback is welcome in [discussions](https://github.com/fuzdev/gro/discussions).

The runtime's docs:

- [runtime](runtime.md) (this doc) - why it exists, the model in a page, the design, and the roadmap
- [authority](authority.md) - the security model: grants and the policy file, run kinds, what holds by construction, the threat model, imports or globals
- [box](box.md) - the kernel layer under the runtime's checks: what Landlock and seccomp hold and can't, the one namespace the box takes, the box a command of Gro's runs in and the one designed for each child, the terminal, and limits
- [run](run.md) - the reference for `fgro run` as built today: what loads, the globals, errors, and exit codes
- [eval](eval.md) - the reference for `fgro eval` as built today: the engine and its limits, determinism, the protocol, the box, the driver, the serializer, the envelope, the graph, and exit codes
- [host modules](host_modules.md) - the imports that reach the OS: `fs`, `spawn`, `net`, `watch`, and the process modules, how to hand them down, and the Node implementation's gaps
- [resolver](resolver.md) - how module specifiers resolve, and the departures from Node
- [typescript](typescript.md) - the type strip: erasable syntax only, and what's refused
- [comparison](comparison.md) - against Node, Deno, and Bun, with the measured startup and size
- [glossary](glossary.md) - the terms these docs use, each defined once
- building and installing it - [install](#install), below, and the README's [Develop](../../README.md#develop) section

Gro is becoming its own JS runtime, a Rust binary that embeds V8,
built on web standards instead of Node compat,
with granular control over what code can reach.
As designed, a run is confined by default:
its code reaches only what it imports or is handed,
so what a run can reach is a static fact about its import graph, its [policy file](glossary.md#policy-file), and any flags the run is given,
reviewed like a lockfile.
Runs can be [hermetic](glossary.md#hermetic) too, with clock and randomness fixed or denied,
so what a run depends on is known and it's designed to replay and cache (`fgro eval` does today, with the gaps [eval](eval.md#the-graph) lists).
It's designed to make what your dependencies and your agents can reach visible, reviewable, and bounded.

Its primary verb is `gro run <file>`,
which runs one module graph with no ambient authority,
reaching nothing the graph didn't import or the policy file didn't accept.
`gro eval` is its confined one-shot sibling, evaluating a module to data.
Until the binary replaces the npm package, the TypeScript version keeps working on Node
and the binary is named `fgro`,
so these docs say `gro` for the design and `fgro` for what's built today.

It starts with `gro eval`, then runs Gro's own workflow (tasks, gen, and config,
[below](#gros-workflow-on-the-runtime)),
and grows into the toolchain for the whole Fuz stack,
including the server side of [fuz_app](https://github.com/fuzdev/fuz_app),
which keeps supporting Node and Bun beside it.

## Why

The runtime started as an exit.
The Fuz stack ran on two runtimes it doesn't own,
Node for Gro and the tools it wraps like SvelteKit and Vite,
and Deno for scripts, tests, and evaluating config it didn't trust under permission flags.
When [Cloudflare bought Deno](https://deno.com/blog/cloudflare) and put the runtime on bug and security fixes
until its end of life in October 2027, everything on it had to move.
The scripts and tests went to Node.
The sandbox was the part worth owning,
since a security boundary is the last thing to leave on a runtime with a fixed end of life,
and the first thing to want by default instead of by flag.

That forced the question of what runtime the stack wants, and the answer was smaller than either.
Gro already runs the stack's TypeScript in process (tasks, gen files, and config) through its Node loader.
The runtime is that loader becoming the host, on the pieces that outlive the sale
([rusty_v8](https://github.com/denoland/rusty_v8), which Cloudflare keeps maintaining,
and [deno_core](https://crates.io/crates/deno_core) as a bootstrap, kept thin, as [Design](#design) says).
One runtime we own in place of two we don't, and one embeddable host for the toolchain.
It also couldn't be Deno with different defaults:
the model the stack wants, authority by import with nothing ambient,
is one Deno can't make its default without breaking its users
([compared to Deno](#compared-to-deno-node-and-bun)).
The sandbox was built first, as `fgro eval`, long before Gro runs on itself.

It isn't a Deno replacement for everyone.
Code that needs Node keeps running on Node,
and the Node tools the stack still uses run as subprocesses until each is replaced.
It isn't only a JS sandbox either, since V8 runs WASM and the same grants confine
any language compiled to it (see [polyglot confinement](#what-the-design-opens-up)).

## The model in a page

[authority](authority.md) is the full statement; this is its shape.

- **Authority enters by import.** The host modules (fs, spawn, watch, net, and the process modules) are the only way to the OS,
  each a visible edge in the graph, and no global carries authority: `fetch` is `net.fetch`, an import.
  So a run's reach is inferred from its import graph, and a dependency bump that newly reaches the network
  or a process is a new edge, attributed to the package.
- **The source proposes, the policy file authorizes.** Inference proposes a run's grants and the committed
  policy file (designed, provisionally named `gro.policy.json`) records what a person accepted; `gro check` fails on a widening until the diff is accepted,
  the same cadence as a lockfile. The loader enforces every edge at link time, literal or computed,
  so the policy holds where a static walk can't see.
- **Nothing to declare for the common case.** Reads under the project root, writes to declared write roots,
  and declared public env names run as project code's own imports without acceptance;
  the network, processes, reads outside the project, private env names, and any host module a dependency imports wait for it;
  `.env*`, credential files, and writes under `.git` or `node_modules` are designed to stay denied short of `-A`
  (the box a command of Gro's runs in refuses those writes at the kernel, and leaves `.env*` readable).
- **Three run kinds.** Hermetic (no network or spawn, clock and randomness fixed or denied, every read recorded, replays),
  confined (bounded by grants, the default), and unconfined (`-A` and the named phases that push and use credentials).
  `gro eval` and Gro's gen files are hermetic.
- **Handles, narrowed and handed down.** A host module exports a handle (a directory, a program, `net` bound to hosts)
  that's narrowed on the value and passed as a parameter, so authority is visible at every use site
  and library code takes it as a parameter rather than importing it ([host modules](host_modules.md#where-to-import-them)).
- **A kernel box under it.** On Linux a coarsening of the grant set becomes Landlock and seccomp rules
  for the whole invocation and every child, so an op-check bug, an engine escape, or a spawned tool still lands in a box;
  children get the project read-only and only the declared write roots writable ([box](box.md)).
- **No prompts.** A grant denial fires at link time, before any confined code has written to your terminal
  (scoped checks inside a grant fail at the call and are reported, failing the run, so a `try/catch` can't hide them:
  built for `env`, `fs`, and `spawn`, designed for the modules to come),
  and the fix is a diff and a fixed command, never a question mid-run.
- **The bound against a hostile dependency** is the invocation's whole grant set and the kernel box, not the realm:
  one realm shares its prototypes, so attribution catches overreach and the box bounds the deliberate
  ([threat model](authority.md#threat-model)).

Of all this, the omissions are built (`fgro` has no runtime namespace, no Node compat, no remote imports, and no authority globals),
`fgro eval`'s box is built, the link-time grant check is built, over the builtin modules [host modules](host_modules.md#the-modules) marks built, which are its only host APIs,
the grant flags [run](run.md#usage) lists are built, the run's box is built for every command of Gro's on the binary,
and the rest of the host modules, the other grant flags, the policy file, the seal, and the box for `fgro run` are designed.

## Design

The runtime prioritizes, in order:

1. security, with no ambient authority, confinement as the default, and a kernel box under it,
   measured by a gate suite of escape probes the way speed is measured by the bench
2. web-standards conformance, to the WinterTC minimum common API less its one member that carries authority,
   `fetch`, which is an import here (`net.fetch`) because security comes first;
   graded by its WPT subset, with the harness supplying `fetch` from `net`
3. developer control and ergonomics, with no prompts, nothing to declare for code that only reads its project's files and writes its declared write roots,
   and a way to widen beside every default (`-A`, a blanket net grant, the JIT flag for eval),
   since a boundary that's painful gets bypassed
4. speed, startup first, since short-lived invocations dominate an agent-driven loop,
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
   (the bench reports the comparison, and a person makes the call for now)
5. binary size and memory

Staying simple and small is overarching, since the security story depends on
a surface small enough to audit, where every path out of the run is an import.
Gro runs on V8, as Node and Deno do, so steady-state compute will be at parity, version for version,
and the wins are everything around the engine (startup, loading, caching, work avoided).
Gro trades some hardening for speed and compatibility by default: one realm, intrinsics left mutable, the JIT on for runs.
The defaults are safe against overreach, and the user decides when to widen them.

The runtime is a Rust binary embedding V8 through [rusty_v8](https://github.com/denoland/rusty_v8).
It's bootstrapped on [deno_core](https://crates.io/crates/deno_core)
and Deno's web-standards extension crates, used thinly behind Gro's own seams
so each piece can be rewritten on its own as measurements justify it.
Each bullet marks what's built; anything unmarked past the omissions is designed.

- **Web standards, not Node compat.** The API is designed as the
  [WinterTC minimum common API](https://min-common-api.proposal.wintertc.org/)
  (today `fgro` has the subset [run](run.md#the-global-scope) lists)
  plus a small set of Gro's own host modules
  (fs, spawn, watch, net, and the process modules; built today as their Node implementation, and on `fgro` only the ones
  [host modules](host_modules.md#the-modules) marks built),
  each an explicit import.
  The running process is one module per kind of authority it carries,
  so each is its own import and its own grant ([the process modules](host_modules.md#the-process-modules)).
  No `node:*`, no `npm:`, no CommonJS, no N-API, and no `process` global.
  The break is what makes the security model enforceable.
  Node's API hands every module ambient authority (`process`, `require`, and `node:fs`
  are reachable from anywhere), so a compat layer would bring it back.
  It also declines the years of work Bun and Deno each spent on Node compat.
  Nor is there a runtime namespace, and no global carries authority:
  beyond the web's pure globals, everything is an import, `fetch` included.
  Gro doesn't take web functionality away, it makes it controllable:
  an API that carries authority is an import, never banned,
  and only narrower run kinds like hermetic add constraints, for what they buy.
  Which web APIs Gro implements beyond that set is decided one at a time.
- **Strict ESM resolution, no legacy algorithm** (built). Relative and absolute specifiers
  resolve as URLs with mandatory extensions, with no probing, no directory index, and no `main`.
  Bare specifiers resolve through a package's `exports` with a fixed condition set
  (`gro`, `import`, `default`, and `development` in a run of `fgro <command>` unless `NODE_ENV` is `production`), walking `node_modules`, and `#` specifiers through the nearest `package.json`'s `imports`,
  to files only.
  A package without `exports`, or one that's CommonJS, fails loudly with the reason and the fix.
  Code comes from files on disk only, never `https:`, `jsr:`, or `npm:` imports,
  and Gro never pre-bundles a project's dependencies.
  Otherwise it follows Node's algorithm, departing where Node guesses, reads past the project,
  or substitutes a pattern unchecked, and [resolver](resolver.md) has the full rules.
- **TypeScript by type stripping only** (built). [tsv](https://github.com/fuzdev/tsv) strips types in process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Like Node's stripper it takes erasable syntax only, so anything with runtime semantics
  (an `enum`, a parameter property) is refused with its position and a fix, never transformed.
  [typescript](typescript.md) has the rules, the refused list, and the file types.
- **No ambient authority** (the link-time check and the grant flags [run](run.md#usage) lists built; the other grant flags and the seal designed). The grant set is a constructor argument of the runtime,
  empty by default, not process-level flags checked at call time.
  Host modules are imports and the module loader is the boundary,
  so an ungranted module fails at link time, not at the first call.
  The network is an import too, never a global, and the global object is sealed after bootstrap
  with named exceptions ([authority](authority.md#imports-or-globals) has the trade-off and the how-to).
- **A kernel box.** On Linux a coarsening of the grant set also becomes kernel rules for the whole invocation,
  installed before V8 starts a thread and inherited by every child process.
  Installed for `fgro eval`, as a fixed box with no grants to enforce yet, and for every command of Gro's on the binary, as the run's box,
  derived from the command's closure and the grant flags; designed for `fgro run`, and for each child inside the run's
  ([eval](eval.md#the-box) has eval's exact walls; [box](box.md) has what each layer holds, the one namespace the box takes, and the run's walls).
- **Native code: first-party crates linked in, WASM as the portable path, FFI as a grant.**
  blake3 and tsv are exposed as builtin modules (`@fuzdev/gro/blake3.ts`, `@fuzdev/gro/tsv.ts`), with no boundary to cross.
  Third-party native code arrives as WASM, or as an embedder's own host modules.
  FFI (calling a native library from JS) is planned as a grant, denied by default,
  with the library loaded in its own confined process, out of reach of the runtime's memory and grants.
  Its calls cross a process boundary, so chatty native work belongs in WASM
  ([polyglot confinement](#what-the-design-opens-up)) or an embedder's host module.
  In-process loading isn't currently planned, and feedback on it is welcome in
  [discussions](https://github.com/fuzdev/gro/discussions):
  if a measured need brings it (chatty calls, callbacks into JS, a library tied to its process),
  it comes as a grant of its own, never part of `-A`,
  shown in the policy diff as leaving the runtime's checks, with the box still around the process.
- **A toolchain, grown by need.** Run and eval (built) and tasks come first,
  then serving (TCP, TLS, WebSocket) so fuz_app's TS backends run on it,
  a test runner, bundled dev and build, and types emitted from the binary.
  tsv's formatter joins it as linked crates (today only its type strip is in `fgro`). Nothing lands before a consumer needs it.
- **Embeddable** (designed). `gro_runtime` is a library other Rust binaries embed,
  with a registration API for their own host modules, and the `gro` binary is one embedder.
  The registration API is Gro's own, not deno_core's,
  so embedders don't break as the core underneath is rewritten.
  Grants are a constructor argument, so an embedder is confined without flags.
  The crates publish when the binary replaces the npm package.
- **One global binary, pinned per project** (designed). A project declares a compatible range under
  `engines.gro` in `package.json`, and the binary will fail hard outside it.
  Svelte, Vite, and SvelteKit stay the project's own dependencies, so the binary owns Gro,
  never the project's compiler or framework.
- **Startup.** A snapshot of the host (built) and lazy host init,
  built for the builtin modules: a module's script is outside the snapshot, compiled and run when a run first imports its module,
  so a run pays for the ones it imports.
  A run or an eval that is over exits without tearing its runtime down (built), leaving the OS to reclaim it.
  A code cache entry is code to V8, so who writes the cache is the whole defense,
  and confined runs read none: Gro's own code is embedded in the binary, its types stripped when the binary is built,
  and compiled from that copy when a run first imports one of its modules (built for `fgro run` and `fgro <command>`),
  and `gro eval` and hermetic runs never read a cache.
  A writer outside the box, compiling from source itself, is the route if a measurement on third-party graphs shows a cache worth it,
  and a cache for unconfined runs is undecided ([box](box.md#open)).
- **Fewer platform-isms** (designed). URL resolution with no path separators (built) and argv-only spawn
  (never a shell string, as the Node `spawn` host module already is) keep platform differences in a handful of files.

## Compared to Deno, Node, and Bun

[comparison](comparison.md) has the full account,
with the design differences from Deno in prose, [what moving a script from Deno changes](comparison.md#moving-a-script-from-deno),
notes on Node and Bun, and the same ground as tables by axis, with the bench's measured startup and size.
The ingredients are mostly shared.
The difference that makes Gro a new runtime rather than a set of Deno flags
is one Deno can't make its default without breaking its users:
authority enters by import, with nothing ambient
(no runtime namespace, no `process`, no `require`, no `node:*`),
so what a run can reach is a fact about its code, not its command line,
with no exception for the network.
The second is that the module graph is closed and on disk, never fetched at run time.
Both are guarantees by omission, so they hold for every run,
where Deno's equivalents are flags that hold for the runs that pass them.
The rest, like the kernel box, no prompts, and deterministic runs,
are defaults Deno could adopt without breaking anyone, and Gro's design turns them on;
with Deno in maintenance until its end of life in October 2027 ([Why](#why)), we don't expect them to land there.
Until the box lands for `fgro run`, its checks are, like Deno's, in-process checks in a process that holds your user's authority,
and there the difference is where authority enters, not yet how strongly it's contained;
a command of Gro's on the binary already runs in the kernel's box.

## What the design opens up

- **Every import of authority attributed.** With the network an import, the policy diff names the package
  that newly imports it, for every kind of authority alike (what a root hands to a library shows in the root's code, not the diff), and `const {fetch} = net` keeps the web's
  spelling inside a module; the error for a free `fetch` names the import.
- **Deterministic runs.** Clock and randomness stay the web's globals,
  set per run kind (real, denied, or fixed) rather than imported,
  and hermetic runs pin the time zone and locale,
  so an eval or other hermetic run replays exactly
  (for one that awaits reads, once the host orders async completions, an [open question](authority.md#open-questions)).
  Hermetic runs are designed to see the project at a fixed root, never this machine's absolute paths,
  so their output is the same on every machine (`fgro eval` doesn't yet: its `import.meta.url` is the real path).
- **Memoized runs.** A hermetic run declares the inputs it may read,
  and its output is keyed by the runtime, its module graph, and the files it actually read,
  so rerunning one is a cache probe (Gro's `gen --check` becomes one).
- **One module graph.** The loader's graph is the only graph.
  It keys caches and invalidation, drives watch,
  and lets a tool list its entries by parsing instead of executing them (Gro's task listing).
- **Cheap separation.** V8 isolates and contexts are cheap in one process,
  so independent runs (Gro's gen files) can run in parallel, each with its own grants under the process's one kernel box, with no process spawn,
  and trust tiers (project, ecosystem, third party) could get their own frozen intrinsics
  (per-tier contexts, an opt-in on the roadmap's future list).
- **Polyglot confinement.** V8 runs WASM, and a WASM module imported as an ES module
  declares its capabilities by its imports, the same thing a JS module's imports are
  (one instantiated from bytes at run time gets exactly the import object the JS hands it, which is handing, not a grant).
  So one policy file, one inference, and one kernel box cover every language that targets WASM,
  and a tool, a plugin, or an agent's script written in Rust, C, Go, Zig, or Python
  runs under the grants its imports declare, beside the TS that calls it, in one process.
  The WASI shape maps onto it directly, with the grant set as the host side of the module's world
  and read roots as its preopens.
  This is where code treated as hostile can go instead of under a looser grant,
  which is what lets the runtime's scope grow past JS without widening the boundary;
  WASM needs the JIT, so against an engine exploit it leans on the box more than jitless JS under `gro eval` does.

## Open questions

Security is the north star, and neither it nor speed is finished.
The design's open questions live with the layer they belong to:

- the authority model's, in [authority](authority.md#open-questions): keeping an agent out of the policy file,
  deterministic scheduling for replay, interposition within one realm and how freezing the intrinsics opts a package out,
  free names through the global's prototype, and which files under the project count as secrets
- the kernel layer's, in [box](box.md#open): the network for a run granted it, a code cache for unconfined runs,
  hard links, browser tests, and the platforms
- **how a package's reach into the project is accepted**: a package's bare import that resolves into the project root
  is refused (built), so a workspace package that imports the app by name, and a tool package that imports the project's config by name,
  wait on a policy-file acceptance naming the package or on workspace support ([resolver](resolver.md#departures-from-node))
- **whether eval's heap cap bounds one large young object**, which today only its memory cap does ([eval](eval.md#the-engine))

## Gro's workflow on the runtime

Gro's task runner, gen, and config move onto the runtime at Gro on Gro ([roadmap](#roadmap)),
its first large consumer and the shape other tools built on it can take.
As designed, except where marked built:

- `gro task <path>` is the workflow layer that carries the original TS Gro API forward,
  running project tasks by full path for explicit resolution,
  while the builtins stay first-class subcommands (`gro check`, `gro dev`) (built on the Node Gro, and on `fgro` for a command that needs no [fallback](glossary.md#fallback)).
- A task runs under the union of what the invocation reaches,
  its own graph plus the tasks it invokes by a literal name (`invoke_task`), since every task holds the shared context (config, `invoke_task`).
  Each builtin's `host` is cut from that union by its own grants, a [guardrail](glossary.md#guardrail) against Gro's own mistakes,
  while the project's code in the same realm holds the whole union, which only the native checks and the run's box bound
  ([threat model](authority.md#threat-model)).
  A Node tool the invocation spawns is held by the run's box (built), by paths and the network, not by the grant set.
- `gro check` is the project's check: it infers each task's and gen file's grants,
  diffs them against the committed policy file, and fails on a widening until `gro policy accept` records it.
  On the Node Gro it runs advisory first, since Node enforces none of it.
- Gen files are hermetic by default, with each one's run kind shown in the task listing,
  so a gen file that stops being hermetic is a reviewed diff, and `gro gen --check` becomes a cache probe.
- `publish`, `deploy`, `commit`, and `changeset` push and use credentials, so they're named unconfined phases.
- The builtins' grants live in Gro: each builtin's `task.grants`, generated into a table the binary carries
  and reads into a [command's closure](glossary.md#commands-closure) before any JS runs
  (built; [task](task.md#a-builtins-host-and-grants)).
- `fgro <command>` runs a command of Gro's on the binary (built, in the run's box):
  every first argument but the binary's own (`run`, `eval`, `--version`, `--help` and `help`, and the internal `make-snapshot` and `make-embed`) is one,
  bare `fgro` lists them, and `fgro -v` prints Gro's version while `fgro --version` keeps the binary's line.
  Its pre-pass, the binary's native step before any thread starts, reads the command's closure,
  the grant flags after the command word, and the project at the working directory,
  which must hold a project's `package.json`.
  The command exits as a run does, its code Gro's, `2` when its pre-pass refuses it, and the Node Gro's when it falls back
  ([run](run.md#errors-and-exit-codes) has every case).
  The project must also install, in their ranges, the peer dependencies of Gro's that its embedded code imports,
  since those resolve from the project's `node_modules` ([resolver](resolver.md#gros-library)),
  so in a project that installed none even bare `fgro` fails, naming the fix.
  The command then runs Gro's CLI from an entry it generates at `gro:main`, the project's origin,
  with the closure's grants joined to the flags' and packages matching the `development` condition unless `NODE_ENV` is `production`.
  It runs in the run's kernel box, which its pre-pass and the steps before V8 put up ([box](box.md#how-it-goes-up)),
  so the tools those grants pass the whole environment (svelte-kit, vitest, eslint, svelte-check, tsc, and npm under `sync`)
  run the project's code bounded by the box's rules rather than by the grants ([box](box.md#what-a-commands-box-admits)):
  under `check`, `test`, `typecheck`, `lint`, and `gen --check` the project is read-only to every process in the run but for the tool directories,
  `format --check`, which starts no tool, writes nothing at all,
  and `gen`, `format`, and `sync` write the project's top-level entries but `.git`, `node_modules`, and the other dot-entries.
  The declared differences from the Node Gro follow, first those of the binary, then those of the box;
  the verb's differential (`crates/gro_cli/tests/verb_differential.rs`) holds the binary's output for a set of commands to what the Node Gro prints,
  normalizing only the differences here that each case names.
  Gro's logger colors its output even under `NO_COLOR` or when piped.
  `fgro --help` and `fgro help` print the binary's own help, where `gro --help` lists the builtins,
  and `fgro --version` the binary's own line, where `gro --version` prints Gro's version, which `fgro -v` prints.
  The console prints a string inside an array or object with double quotes, where Node's prints single ones,
  so a list Gro logs, like `gen`'s files, reads differently.
  A forwarded section that turns off a check its command runs, like `fgro check -- gro gen --no-check`,
  is refused before anything runs, exit 2, where the Node Gro writes,
  since the closure keeps an invoke's `check` mark whatever the command line forwards,
  and the refusal names the command that writes on the binary (`fgro gen`) and the whole command on the Node Gro.
  Gro's `library_load.ts`, which reaches Node's builtins, isn't in the binary's embedded code, so a genfile that imports it, like a library's API docs genfile, fails to load, the error naming the Node Gro;
  and `sveltekit_tsconfig_is_unsynced` fails on the binary, since it reads `tsconfig.json` through TypeScript, whose package has no `"exports"` and is CommonJS, both refused ([resolver](resolver.md#specifiers)).
  A project's typecheck reads the types of the Gro it installed while the command runs the binary's embedded code.
  Another installed version falls back, so the two can differ when the installed Gro has the binary's version but not its code,
  like a build of a later commit before the version moves, or when the project installed no Gro, whose typecheck then finds no Gro types.
  A run that fails after the resolver refused a `node:` specifier, caught or not, ends with a hint naming the cause
  and the whole command on the Node Gro, which confines nothing (`node_modules/.bin/gro` and the command line, its grant flags stripped),
  a [review signal](glossary.md#review-signal) the run's code controls: project code can print it at will by catching a `node:` import and failing.
  The box adds these, each a cost of holding the tools where the box holds them:
  - A tool reaches no network but the run's own loopback, so a test that connects to a service outside the run, like a database on the host, fails, its connection refused or its network unreachable.
    Where the system restricts unprivileged user namespaces, every internet socket is refused instead, loopback's included,
    so a test that binds `127.0.0.1` fails too, and a `note:` line after the box goes up says why.
  - A tool reads nothing under the home directory but what git reads there (its global config, its ignore and attributes files, and the files the config includes or names)
    and the programs the run pins, like `node`'s file where a version manager installed it there, beside the project's own trees wherever they sit,
    so one that reads its own config or cache there finds it unreadable, failing with `EACCES` or running as though there's none,
    and git's credential store stays unreadable unless your own git config names it as a file git reads.
  - A git config the box can't read as git does, or a `.git` file that isn't a linked worktree's or a submodule's leading back to the project,
    is refused before the box goes up, exit 2, for a command that may start a program.
    A project inside a repository whose `.git` is at the home directory or above, like a dotfiles repository, runs, but git run there fails, unable to read it.
  - In a run that may start a program, or that `--allow-temp` grants, `TMPDIR` names a directory private to the run, which a later start removes once the run has ended,
    and so do `TMP` and `TEMP` where they were set,
    and `XDG_DATA_HOME`, `XDG_CACHE_HOME`, and `XDG_STATE_HOME` name directories inside it,
    so a tool that keeps state there (Vitest's token among it) gets a fresh, private place, nothing under the home directory is touched for it,
    and a tool that caches in either place starts cold every run.
    An OS temporary directory inside the project is refused for such a run, exit 2, the refusal naming the variable that set it, or saying it's the default.
  - A write outside the command's write roots is refused at the kernel:
    in Gro's own code a `permission_denied` `FsError` whose cause names where the box lets the command write and what stays read-only,
    and in a tool `EACCES` or `EPERM`, after which, when the run exits nonzero, a closing `note:` line names the box's network and writes and the flags that widen it
    ([box](box.md#what-a-commands-box-admits) has both texts and when each appears).
    So a genfile's new top-level output fails, as does any write under `.git`, `node_modules`, or a dot-entry,
    where the Node Gro writes each.
  - Nothing in the project executes outside `node_modules`, so a script there run by its path fails with `EACCES`, while its interpreter runs it (`node scripts/x.js`).
  - A tool reads `/proc` as on Node, but the environment, memory, and working directory of a process outside the run are refused.
  - The run is not dumpable, so attaching to it (`perf -p`, `gdb -p`) is refused; starting `fgro` under `perf` or `gdb` works.
  - The `core` limit is 0 for the run and every tool, so a crash writes no core file.
  - A setuid or setcap program, like `sudo` or `ping`, runs without its privileges, since the box sets `no_new_privs`.
  - A tool can't make a user namespace, so a browser that sandboxes itself with one, like Chromium under a test, fails to start.
  - A tool can signal only processes inside the run, never one outside it, like an editor or a dev server it didn't start.
  - A socket on stdin, stdout, or stderr that isn't a connected unix stream is refused before anything runs, exit 2,
    so an inetd-style launch fails, while a pipe, a file, a terminal, `/dev/null`, and a connected socket pair, like Node's `spawn` makes, pass.
  - `DENO_STARTUP_PHASES`, `DENO_LOG_LAZY_LOAD`, and `DENO_SNAPSHOT_IMPORT_GRAPH`, which the vendored `deno_core` reads, are removed for the run and its tools.
  - `-A` is refused, exit 2, after the command word as
    `error: -A would run a command of Gro's unconfined, and the binary runs each in a kernel box: the project's Node gro is the unconfined spelling (node_modules/.bin/gro format --check)`,
    and before it as `-A comes before the command, and the binary runs each command of Gro's in a kernel box, which -A can't lift: …`, with the same command;
    before `run`, as in `fgro -A run x.ts`, the refusal says grant flags go after the command word, like `fgro run -A`.
  - A `node` on `PATH` that's a script, like an asdf or nodenv shim, is refused before the box goes up, exit 2, naming the shim and the fix
    (the real `node` first on `PATH`, or `--allow-run-in=` naming the version manager's directory),
    since the shim reads its manager's tree under the home directory, which the box refuses.
    A native shim, like volta's, can't be told apart from `node`, and its tools fail inside the box.
  - A `node_modules` that's a symlink to another checkout's `node_modules` is followed, read and executed there, with a `note:` line naming where it leads,
    and Gro's own code reads through it too, read-only;
    its packages resolve within it, reading nothing of the checkout holding it,
    and its tool directories are made, and anything written there, only when an `--allow-write=` names it, so a tool that needs one fails without the flag.
    A `node_modules` linked to anything else is refused before the box goes up, exit 2.
  - A binary built with a plain `cargo build`, which has no snapshot, refuses every command of Gro's it would run in its box, exit 1,
    since it reads its own sources from its build tree, which no rule of the box admits.
- A command the binary leaves to Node, like `dev`, `build`, `clean`, `check --build`, a project task, or `task`'s listing,
  or one in a project that installed another version of Gro, or any command where Landlock is missing, off, below its sixth ABI (Linux 6.12), or refused by a seccomp filter, which can't put up the box,
  runs on the Node Gro the project installed (built):
  one `note:` line says why, then `node`, found on `PATH` by its absolute entries with `node_modules/.bin` ones skipped,
  replaces the process, running `node_modules/@fuzdev/gro/dist/invoke.js` with the command line as written
  and Gro's loader registered as the Node Gro registers it, `development` condition included unless `NODE_ENV` is `production`.
  The host's fallback names its reason, like
  `note: fgro check runs on the Node gro in node_modules/@fuzdev/gro <version>, which confines nothing, because Landlock isn't available here (the kernel has none, or a seccomp filter refuses its calls), which the kernel box needs at ABI 6 (Linux 6.12) or later`.
  `GRO_FALLBACK=command` refuses a fallback the project's files decide, the version, and one the host decides, the kernel's, exit 2;
  a fallback the command decides still runs the project's installed `dist`, which confines nothing
  ([authority](authority.md#what-the-first-release-is-for)).
  So a project's policy file is designed to hold its own tasks alone,
  and `gro check`, which spawns Node tools through Gro's own tasks, leaves the project nothing to declare for them.

## Roadmap

The first target, retiring Deno from the Fuz stack before its end of life
in October 2027 (see [Why](#why)), is met.
Until Node leaves much later, third-party Node tools (Vite, vitest, eslint, svelte-check)
run as Node subprocesses; the design puts them under the invocation's kernel rules.
The runtime and Gro's workflow on it are two tracks, and they meet at Gro on Gro,
where the host modules and grants land with their first consumer.

### The runtime

- [x] Rust workspace and a benchmark harness that times Gro beside Node, Deno, and Bun
      on the same workloads, the baseline every milestone is measured against
- [x] `gro run` (as `fgro run` for now), running ESM from disk on V8 with web-standard globals
      and a startup snapshot
- [x] TypeScript by tsv's type strip, in process
- [x] strict package resolution, through `node_modules` by `"exports"` up to the project root,
      with `"type": "module"`, and `#` imports to files
- [x] origin containment in the resolver ([resolver](resolver.md#departures-from-node)),
      keeping every module inside its project's or package's root,
      and the loader reading only the file the resolver found
- [x] `gro eval` (as `fgro eval` for now), a confined, deterministic one-shot eval of a TS module to JSON
      under the box, for any tool that evaluates config;
      zap evaluates its config on it, replacing Deno for config eval in the Fuz stack
- [ ] the security gate, a suite of escape probes run under every grant set,
      with every web global and member classified by the three tests for [authority](glossary.md#authority-globals) as its oracle,
      and handle probes (a narrowed handle can't be widened back, one handle can't reach another,
      a transferred or revoked one is dead in its old holder),
      grown from `fgro eval`'s acceptance tests and passed by every milestone after,
      with probes for a symlink and a hard link out of the roots, each killed syscall, a same-realm `then` patch,
      an escape sequence in confined output, and a child reaching a unix socket,
      with the type strip fuzzed over real packages, since it runs on `node_modules`
- [ ] startup: Gro's own code embedded in the binary, stripped at build time and loaded when a run first imports it (built for `fgro run` and `fgro <command>`),
      and lazy host init (built for the builtin modules, each run on its first import), with no code cache for confined runs
- [ ] the seal on the global object, with named exceptions, and a free `fetch`'s error naming the import
- [ ] the host modules (fs, spawn, watch, net, and the process modules), the grants
      (built so far: the link-time check, the modules [host modules](host_modules.md#the-modules) marks built, and the flags [run](run.md#usage) lists),
      and the policy file, enforced, with the box under every run and its children
      (built for every command of Gro's on the binary, designed for `fgro run` and for each child's own box,
      [box](box.md#the-box-for-runs-and-children)), landing with Gro on Gro
- [ ] each tool in a box of its own inside the run's, writing only the directories the grants table names for it
      ([box](box.md#a-box-for-each-tool))
- [ ] the first release, Gro running on this runtime, for Linux x86_64, once each tool has its own box;
      the tool that installs it records the binary's hash as a pin,
      which the caller of `gro eval` verifies before every spawn
      (no binary release is attached yet, and `fgro` isn't released on its own)
- [ ] macOS and Windows builds, each with its own sandbox design
- [ ] binary-only, where `fgro` becomes `gro`, the npm package carries types, and the crates publish
- [ ] serving and types from the binary, so fuz_app's server side runs on Gro

### Gro's workflow

- [x] on the Node Gro, `gro task <path>` replaces the bare `gro <name>` task search
      and the loader is strict, so the scripts the Deno exit ported were written once, in the final shape
- [ ] on the Node Gro, `gro check` infers each task's grants
      and diffs them against the committed policy file, labeled advisory,
      since Node enforces none of it, and reports each grant the graph no longer imports,
      the source for pruning the policy file
- [ ] on the Node Gro, a lint generated from the authority table,
      refusing free authority globals in code meant for the runtime, a portability check
- [x] the Deno exit: config eval is on `fgro eval`, and the stack's other Deno uses moved to Node,
      `deno task` becoming npm scripts or gro tasks
- [ ] Gro on Gro, with tasks, gen, and check running on the runtime and enforcing the policy file
      (built so far, `fgro <command>` runs the builtins on the runtime, held to their closure's grants, in the run's box,
      unless the command needs a fallback to Node, as a project task does; until the policy file the CLI passes every host module)
- [ ] bundled dev and build on the same host, and Vite leaves the loop
- [ ] a test runner on the runtime, and with serving, Node leaves the stack's toolchain

Future (unknown order):

- FFI as a grant, in its own confined process
- the `fixed` and `seeded` determinism modes beyond eval's `deny`,
  and memoized hermetic runs, gen first
- owning the core, replacing deno_core's pieces one at a time, measured
- `net.WebSocket` and `net.EventSource` on the `net` handle, and `crypto.subtle`, each as a consumer needs it
- other web APIs that carry authority (storage, workers, by-name channels), each an import if Gro implements it
- size: filtered ICU data, V8 build flags
- V8 contexts per package or trust tier with frozen intrinsics, as an opt-in hardening tier, never the default

Maybe later or never:

- a custom V8 build

## Install

No binary release is attached yet, and `fgro` isn't released on its own:
the first published version will be Gro running on this runtime ([roadmap](#roadmap)),
the binary attached as `gro` to the GitHub Release of the npm package's own tag, `v<version>`.
Until then, build it from source, for Linux x86_64 ([the README](../../README.md#develop)),
and see [crates/CHANGELOG.md](../../crates/CHANGELOG.md) for what the binary built from `main` does;
`fgro eval` needs Linux 6.12 or later too ([eval](eval.md#usage)),
and below it `fgro <command>` runs the Node Gro instead ([Gro's workflow](#gros-workflow-on-the-runtime)).

```bash
cargo xtask build --release   # makes target/gro_snapshot/release/fgro, its startup snapshot and Gro's own code embedded
```

Install it to a path you update deliberately, since a caller that pins the binary
records the hash of that file.
zap takes the binary by absolute path on every command that evaluates a config,
from `ZAP_FGRO` or `--fgro` (under `sudo`, which resets the environment, only `--fgro`),
and records its pin with `zap lock`, which hashes that binary (blake3)
and writes the hash to the project's `zap.lock`:

```bash
install -D -m 755 target/gro_snapshot/release/fgro ~/.zap/bin/fgro
export ZAP_FGRO="$HOME/.zap/bin/fgro"   # bash / zsh; in fish: set -gx ZAP_FGRO ~/.zap/bin/fgro
zap lock   # in the zap project's directory
```
