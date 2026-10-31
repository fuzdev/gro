# runtime

> Gro's experimental JS runtime: a Rust binary on V8 with granular control over what code can reach;
> why it exists, the security model, the design, and the roadmap

> **status**: experimental, with the direction settled and the design in progress.
> Today `fgro run <file>` (renamed `gro` when ready) runs a JS or TypeScript module
> from disk on V8 (x86_64 Linux only) using Deno's libraries,
> with strict resolution and origin containment ([run](run.md));
> the grants, the policy file, and the kernel backstop below are designed, not built,
> so no grant is enforced yet; the [roadmap](#roadmap) marks what's built.
> The design doesn't have every answer yet ([open questions](#open-questions)),
> and feedback is welcome in [discussions](https://github.com/fuzdev/gro/discussions).

The runtime's docs:

- [runtime](runtime.md) (this doc) - why it exists, the security model, the design, and the roadmap
- [run](run.md) - the reference for `fgro run` as built today: what loads, the globals, errors, and exit codes
- [resolver](resolver.md) - how module specifiers resolve, and the departures from Node
- [typescript](typescript.md) - the type strip: erasable syntax only, and what's refused
- [comparison](comparison.md) - against Node, Deno, and Bun, with the measured startup and size
- [glossary](glossary.md) - the terms these docs use, each defined once
- building it - the README's [Develop](../../README.md#develop) section

Gro is becoming its own JS runtime, a Rust binary that embeds V8,
built on web standards instead of Node compat,
with granular control over what code can reach.
By default a run is confined.
Its code reaches only what it imports or is handed,
the way a function reaches only the bindings in scope,
plus any global the policy file accepts,
so what a run can reach is a static fact about its import graph and its policy,
reviewed like a lockfile, and the common case has nothing to declare.
Runs can be hermetic too, with clock and randomness fixed or denied,
so what a run depends on is known and it replays and caches by construction.
It's built for code you didn't write, like your dependencies and your agents.

Its primary verb is `gro run <file>` (currently named `fgro` until ready),
which runs one module graph with no ambient authority,
reaching nothing the graph didn't import or the policy file didn't accept.
`gro eval` is its confined one-shot sibling.
`gro task <path>` is the workflow layer that carries the original TS Gro API forward,
running project tasks by full paths for explicit resolution,
while the builtins stay first-class subcommands (`gro check`, `gro dev`).

It starts with `gro eval`, then runs gro's own TS surface (tasks, gen, config),
and grows into the toolchain for the whole Fuz stack,
including the server side of [fuz_app](https://github.com/fuzdev/fuz_app),
which keeps supporting Node and Bun beside it.
Until the binary replaces the npm package, the TypeScript version keeps working on Node
and the binary is named `fgro`.

These terms carry the rest of this doc, and the [glossary](glossary.md) defines them and the others:

- **host modules** are the imports that reach the OS (fs, process, spawn, watch, net),
  each a visible edge in the graph
- **authority globals** are the web globals that reach the world (`fetch`, `WebSocket`),
  the one authority not declared by import, since a global reaches every module in the graph
- a **grant** is one accepted reach, like a host module, named hosts, or a read root
- the **policy file** (currently `gro.policy.json`) is the committed record of the grants accepted in review
- every run is one of three **kinds** by what it reaches and depends on,
  hermetic (replayable), confined (bounded), or unconfined
- the **kernel backstop** is the grant set enforced again as Linux kernel rules (Landlock, seccomp),
  under the runtime's own checks
- **reach**, **need**, and **use** are three views of what code can do:
  what it could get to (static, from the import graph and the policy file, and the only one enforced),
  what a function asks for (static, in its parameter types),
  and what a run actually did (dynamic, in the access manifest, evidence rather than enforcement)

## Why

The runtime started as an exit.
The Fuz stack ran on two runtimes it doesn't own,
Node for gro and the tools it wraps like SvelteKit and Vite,
and Deno for scripts, tests, and evaluating config it didn't trust under permission flags.
When [Cloudflare bought Deno](https://deno.com/blog/cloudflare) and put the runtime on bug and security fixes
until its end of life in October 2027, everything on it had to move.
The scripts and tests go to Node.
The sandbox was the part worth owning,
since a security boundary is the last thing to leave on an unmaintained runtime,
and the first thing to want by construction instead of by flag.

That forced the question of what runtime the stack wants, and the answer was smaller than either.
Gro already runs the stack's TypeScript in process (tasks, gen files, and config) through its Node loader.
The runtime is that loader becoming the host, on the pieces that outlive the sale
([rusty_v8](https://github.com/denoland/rusty_v8), which Cloudflare keeps maintaining,
and [deno_core](https://crates.io/crates/deno_core) as a bootstrap, kept thin, as [Design](#design) says).
One runtime we own in place of two we don't, and one embeddable host for the toolchain.
The sandbox ships first, as `gro eval`, long before gro runs on itself.

It isn't a Deno replacement for everyone.
Code that needs Node keeps running on Node,
and the Node tools the stack still uses run as subprocesses until each is replaced.
It isn't only a JS sandbox either, since V8 runs WASM and the same grants confine
any language compiled to it (see [polyglot sandboxing](#what-the-design-opens-up)).

## Secure by default

Code starts with nothing and gets only what it's granted.
There's no flag to remember and no mode to opt into.
The default is the confined case, and widening it is a visible act, reviewed like a lockfile change.
As designed, not yet built, a run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared build roots,
  and the public env names it declares
- **only after it's accepted in review:** the network, by named hosts
  (a blanket grant exists, accepted the same way and shown as the widest class),
  spawning processes, reads outside the project, private env names,
  and any host module a dependency imports
- **never short of `-A`:** `.env*` files except through the env module,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`
- **under `gro eval`:** its own module graph, read-only, with no network, writes, env, or spawn,
  and the JIT off unless a flag turns it on
  (with the JIT off V8 has no `WebAssembly` at all, so a config that uses WASM needs the flag)
- **for native code:** never the runtime's process, not even under `-A`,
  since FFI is a grant that loads the library in its own confined process

A script's needs are a static property of its import graph,
so least privilege costs nothing to write.
A file that imports only `fuz_util` and the fs module, and declares no global, can't spawn or fetch.
That's `gro run`'s unit, one file's graph.
`gro task` runs under the union of what the invocation reaches,
since a task holds the context (config, `invoke_task`),
and a Node tool it spawns is confined by the kernel backstop, not the grant set.
The source proposes grants (the host modules it imports, plus a static `grants` literal
naming hosts and the like) and the committed policy file authorizes them,
so a dependency bump that newly imports the filesystem, spawn, or the network fails
`gro check` until the diff is accepted, the same shape as a lockfile change.
The authority globals are the exception: once a run holds the global `fetch`,
a dependency that starts calling it changes no grant.
A planned scan flags it in the diff for review,
but the scan can be evaded and enforces nothing ([imports or globals](#imports-or-globals)).

The policy file is also the guardrail for agents.
What an agent's code may reach sits in one small file a person can audit,
the runtime will enforce it (nothing enforces it today), and a widening is a diff in review.
Keeping an agent from editing that file itself is still open.
Review, harness rules, and a push gate cover the normal case but aren't a structural guarantee,
and a signed acceptance is designed for when it's needed
([open questions](#open-questions)).

Every run is one of three kinds, derived from its grants,
recorded in the policy file, and shown in the task listing,
so a gen file that stops being hermetic is a reviewed diff.

| kind       | network and spawn | clock and randomness | reads                         | replays |
| ---------- | ----------------- | -------------------- | ----------------------------- | ------- |
| hermetic   | none              | fixed or denied      | under the root, each recorded | exactly |
| confined   | by grant          | real                 | by grant                      | no      |
| unconfined | everything        | real                 | everything                    | no      |

Gen files and `gro eval` are hermetic by default.
A grant is always a grant of something specific, so granting individually,
even a blanket network grant, keeps a run confined, with a wider reach shown in the diff.
That includes the authority globals, since `fetch` reaches every module in the graph
but only the hosts accepted.
Only `-A` (`--allow-all`, as in Deno) and the named unconfined phases below run unconfined,
and a grant as strong as `-A`, like spawning a shell, is flagged that way in the diff.

The guarantees come in two kinds, and it's worth knowing which is which.
Some hold by construction, with no setting that changes them.
Authority enters by an import or a declared global, so what a run can reach is a static fact,
though which of its modules ends up holding a capability is not.
There's no `process` global, no Node compat, no in-process FFI, no remote import,
and no runtime namespace to reach around the loader.
A WASM module's imports are its capability declaration the same way.
The rest hold by default, where a policy entry or `-A` widens them and the diff shows it.
These are the read and write roots, the net hosts, the authority globals, the deny overlays,
the empty environment for children, the kernel rules, and whether a run stays hermetic.
A widening of the second kind is a diff accepted in review, and the first kind has nothing to widen.

The exceptions are explicit and labeled.
`-A` is the one blanket escape hatch, never a default.
`publish` and `deploy` push and use credentials, so they're named unconfined phases,
shown that way in the policy diff, with credentials passed to the one process that needs them.
Linux is the design target.
The kernel backstop and every claim that rests on it are Linux's,
and macOS and Windows get sandbox designs of their own before the binary ships there,
never a translation of Linux's.

Security-first runtimes earned a reputation for friction,
with flags on every run, prompts mid-execution, and `-A` as the habit that follows.
Gro has none of those.
A normal run has no flags and nothing to declare,
and the one moment worth a human's attention, when the graph newly reaches the network,
a process, or files outside the project, arrives as a diff with a copyable fix, never a prompt.
The costs are elsewhere, and stated plainly:
no Node compat, strict ESM, and the authority globals absent until declared.

## Threat model

Grants contain overreach.
A dependency, a task, or an agent that reaches for authority nobody accepted
fails before it runs (an unaccepted host module fails at link time,
and an undeclared global isn't there), and the widening shows up as a diff in review.
V8 itself isn't treated as the boundary against code built to escape it.
An engine exploit lands in the kernel backstop,
which applies the same grants to the whole invocation and every child process,
and `gro eval` and FFI each run in a process of their own.
Within one realm, a package's own grants catch overreach and attribute it in review,
but a dependency built to steal can patch a shared prototype
and catch a capability on its way through project code,
so against it the bound is the invocation's whole grant set and the kernel backstop.
Code that must be treated as hostile belongs in WASM or a separate process,
not under a looser grant.
WASM runs with V8's JIT on, so against an engine exploit
what holds it is the kernel backstop, not the engine.
Until the Node tools leave the loop, an invocation that spawns them
is confined only as finely as the kernel backstop, by paths and ports rather than the grant set.
Outside the model are the unconfined phases (`publish`, `deploy`, `-A`),
and on macOS and Windows anything past the runtime's own checks,
until those platforms get a kernel equivalent.

## Design

The runtime prioritizes, in order:

1. security, with no ambient authority, confinement by construction, and a kernel backstop under it,
   measured by a gate suite of escape probes the way speed is measured by the bench
2. web-standards conformance, to the WinterTC minimum common API,
   graded by its WPT subset under a full grant set
3. developer control and ergonomics, with no prompts, nothing to declare in the common case,
   and a way to widen beside every default (`-A`, the globals grant, the JIT flag for eval),
   since a boundary that's painful gets bypassed
4. speed, startup first, since short-lived invocations dominate an agent-driven loop,
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
   (the bench reports the comparison, and a person makes the call for now)
5. binary size and memory

Staying simple and small is overarching, since the security story depends on
a surface small enough to audit, where every path to the OS is an import or a declared global.
Gro runs on V8, as Node and Deno do, so steady-state compute will be at parity, version for version,
and the wins are everything around the engine (startup, loading, caching, work avoided).
Gro isn't security maximalism. The defaults are safe and the user decides when to widen them.

The runtime is a Rust binary embedding V8 through [rusty_v8](https://github.com/denoland/rusty_v8).
It's bootstrapped on [deno_core](https://crates.io/crates/deno_core)
and Deno's web-standards extension crates, used thinly behind gro's own seams
so each piece can be rewritten on its own as measurements justify it.

- **Web standards, not Node compat.** The API is the
  [WinterTC minimum common API](https://min-common-api.proposal.wintertc.org/)
  plus a small set of host modules from [`fuz_host`](https://github.com/fuzdev/fuz_host)
  (fs, process, spawn, watch, net), each an explicit import.
  No `node:*`, no `npm:`, no CommonJS, no N-API, and no `process` global.
  The break is what makes the security model enforceable.
  Node's API hands every module ambient authority (`process`, `require`, and `node:fs`
  are reachable from anywhere), so a compat layer would bring it back.
  It also declines the years of work Bun and Deno each spent on Node compat.
- **Strict ESM resolution, no legacy algorithm.** Relative and absolute specifiers
  resolve as URLs with mandatory extensions, with no probing, no directory index, and no `main`.
  Bare specifiers resolve through a package's `exports` with a fixed condition set
  (`gro`, `import`, `default`), walking `node_modules`, and `imports` for `#` subpaths comes later.
  A package without `exports`, or one that's CommonJS, fails loudly with the reason and the fix.
  Code comes from files on disk only, never `https:`, `jsr:`, or `npm:` imports,
  and gro never pre-bundles a project's dependencies.
  Otherwise it follows Node's algorithm, departing where Node guesses, reads past the project,
  or substitutes a pattern unchecked, and [resolver](resolver.md) has the full rules.
- **No runtime namespace.** Beyond the web's globals, everything is an import.
- **TypeScript by type stripping only.** [tsv](https://github.com/fuzdev/tsv) strips types in process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Like Node's stripper it takes erasable syntax only, so anything with runtime semantics
  (an `enum`, a parameter property) is refused with its position and a fix, never transformed.
  [typescript](typescript.md) has the rules, the refused list, and the file types.
- **No ambient authority.** The grant set is a constructor argument of the runtime,
  empty by default, not process-level flags checked at call time.
  Host modules are imports and the module loader is the boundary,
  so an ungranted graph fails at link time, not at the first call.
  The network is an import too.
  The host's net module exports a `net` handle (`net.fetch`, bound to the accepted hosts)
  that's narrowed and passed down like any other capability,
  so authority is visible at every use site and nothing shadows the web's `fetch`.
  A net grant is inferred from the graph like a filesystem one, naming hosts,
  and a blanket one is never proposed for you, only accepted, shown in the policy diff as the widest class.
  Pure globals (`URL`, `TextEncoder`, streams, `console`) are always there,
  and ops check the scoped cases (reads under a root).
- **No new globals.** The authority globals are a separate, wider grant,
  since a global reaches every module in the graph
  ([imports or globals](#imports-or-globals) weighs the two).
  They're declared, never inferred, and absent until then.
  The global object is the one dynamic channel, and it's made non-extensible after bootstrap,
  so a handed capability can't be stashed there as a new global.
  A package that adds a global needs a policy entry,
  and like any global, the entry's effect covers the whole invocation.
  Existing globals and the shared prototypes stay mutable, as in browsers.
  A reference parked on one never widens what its holder could reach,
  but a module that patches a shared prototype can interpose on calls that pass through it,
  and so obtain capabilities it was never handed.
  Against a dependency treated as hostile the realm isn't the boundary,
  and freezing the intrinsics is a per-tier hardening, not the default.
- **A kernel backstop.** On Linux the grant set also becomes kernel rules for the whole invocation
  (Landlock for the filesystem and TCP, seccomp for the rest of the socket and
  process surface), installed before V8 starts a thread and inherited by every
  child process, so an op-check bug, an engine escape, or a spawned tool still lands in a box.
  The kernel layer is coarser than the runtime's own checks (paths and ports, not
  files carved out of a tree or hostnames), so it backstops them rather than replacing them.
- **Native code: first-party crates linked in, WASM as the portable path, FFI as a grant.**
  tsv is linked in already; it and blake3 will be exposed as built-in modules with no boundary to cross.
  Third-party native code arrives as WASM, or as an embedder's own host modules.
  FFI (calling a native library from JS) is planned as a grant, denied by default,
  with the library loaded in its own confined process.
  There is no in-process FFI, so native code can't reach the runtime's memory or its grants.
  Calls cross a process boundary, so chatty native work belongs in WASM instead,
  where a module reaches only the imports the host hands it
  (see [polyglot sandboxing](#what-the-design-opens-up)).
- **A toolchain, grown by need.** Run, eval, and tasks come first,
  then serving (TCP, TLS, WebSocket) so fuz_app's TS backends run on it,
  a test runner, bundled dev and build, and types emitted from the binary.
  tsv formats, linked in as crates. Nothing lands before a consumer needs it.
- **Embeddable.** `gro_runtime` is a library other Rust binaries embed,
  with a registration API for their own host modules, and the `gro` binary is one embedder.
  The registration API is gro's own, not deno_core's,
  so embedders don't break as the core underneath is rewritten.
  Grants are a constructor argument, so an embedder is confined without flags.
  The crates publish when the binary replaces the npm package.
- **One global binary, pinned per project.** A project declares a compatible range under
  `engines.gro` in `package.json`, and the binary fails hard outside it.
  Svelte, Vite, and SvelteKit stay the project's own dependencies, so the binary owns gro,
  never the project's compiler or framework.
- **Startup.** A snapshot of the host, a code cache keyed by content, and lazy host init.
  A cache entry is code to V8, so the cache directory is written only by gro,
  never by a confined run or its children, and `gro eval` and hermetic runs never read it.
  On a platform without the kernel backstop it stays off until that platform's sandbox exists.
- **Fewer platform-isms.** URL resolution with no path separators and argv-only spawn
  (never a shell string) keep platform differences in a handful of files.

### Imports or globals

Authority reaches code two ways, and they trade differently.

|                    | a host module import (`net.fetch`, fs, spawn)            | an authority global (`fetch`, `WebSocket`)        |
| ------------------ | -------------------------------------------------------- | ------------------------------------------------- |
| granted            | inferred from the import, accepted in review             | declared by hand, accepted in review              |
| reaches            | the modules it's imported into and handed to             | every module in the invocation                    |
| the diff shows     | which package newly reaches it                           | that the invocation needs it, not which code uses it |
| narrowed           | on the value (fewer hosts, a subdirectory), passed down  | to the accepted hosts, the same for every module  |
| when denied        | the import fails at link time                            | it's absent, so `typeof fetch` is `'undefined'`   |
| portable           | through `fuz_host`, which runs on Node too               | anywhere the web API exists                       |
| existing packages  | the ones written to take it                              | the ones that call the global today               |
| in the code        | passed down the call graph                               | reachable from anywhere                           |

Both columns describe code that doesn't patch shared prototypes;
against a dependency that does, the [threat model](#threat-model) says where the bound is.

The import is the default and the narrow path, and where it happens matters:

- Import host modules at the [composition root](glossary.md#composition-root),
  the task or entry module where authority enters,
  so the policy diff points at a few files a person can review.
- Library code takes capabilities as parameters, through small dependency interfaces,
  the way a function takes its dependencies.
  Tests then pass mocks with no module mocking,
  and a library given fs needs no policy entry of its own, since handing is delegation, not a new grant.
- Name a handle's type with `import type`, never `import {type Fs}`.
  Under `verbatimModuleSyntax` the second form keeps the import,
  so the host module still links and the library looks like it reaches fs.
- Hand down the narrowest handle (a directory handle, `net.narrow({hosts})`) rather than the module,
  so narrowing happens at the root, where review sees it.
- `net.fetch` is bound, so it can be handed to any library that accepts an injected `fetch`,
  and `const {fetch} = net` gives the bare name in one module.

A single-file script is its own root and imports directly.
The cost is that the policy sees [reach](glossary.md#reach-need-and-use) only at the roots.
What a library needs shows in its parameter types, what it used shows in the access manifest,
and what the root hands to whom is in the root's code, not the policy file.
A grant limits what code can obtain, never what it's given,
so even enforced per package it bounds the importer, not the code the importer hands to.

The globals grant is for code that can't take an injected value,
like a package that calls the global `fetch` and can't be changed,
or code that must also run unchanged in browsers and other runtimes.
Its cost is scope.
One realm has one global object, so granting `fetch` for the one package that needs it
gives it to every module, a dependency that starts calling it later included,
and the policy entry names the need while its effect covers the invocation.
A planned scan records which packages reference each authority global by name,
so a package that starts calling `fetch` shows in the diff for review.
It's a review signal, not a boundary.
Code that reaches a global indirectly (`globalThis[name]`, `Reflect.get`, a reference handed in)
gets past it, though dynamic access to the global object is itself flagged,
and nothing it records is enforced until globals are scoped per package,
an [open question](#open-questions).

Clock and randomness stay globals for a different reason.
`Date.now()` and `Math.random()` are everywhere and can't be removed,
and determinism is a property of the whole run,
so they're set per run kind rather than granted.

## Compared to Deno, Node, and Bun

[comparison](comparison.md) has the full account,
with the design differences from Deno in prose, notes on Node and Bun,
and the same ground as tables by axis, with the bench's measured startup and size.
The ingredients are mostly shared, and the structural difference from Deno is that
authority enters by import, with no namespace to reach around the loader,
and the declared globals the one exception.
In one line, Deno's lockfile pins which code runs, and its config can hold a hand-written permission set for the process,
while gro's policy file pins what that code can reach, per import chain for host modules
and per run for the globals, inferred from the graph where it can be,
and is treated like the lockfile, committed, diffed, and reviewed.

## What the design opens up

- **Grants that read like the web.** `typeof fetch` stays the feature check the web already uses,
  and the error for an undeclared one names the fix, the import first and the global grant second.
- **Deterministic runs.** Clock and randomness stay the web's globals,
  set per run kind (real, denied, or fixed) rather than imported,
  and hermetic runs pin the time zone and locale,
  so an eval or gen run replays exactly
  (for gen, once the host orders async completions, an [open question](#open-questions)).
  Eval and gen see the project at a fixed root, never this machine's absolute paths,
  so their output is the same on every machine.
- **Memoized gen.** A gen file declares the inputs it may read,
  and its output is keyed by the runtime, its module graph, and the files it actually read,
  so `gro gen --check` becomes a cache probe.
- **One module graph.** The loader's graph is the only graph.
  It keys caches and gen invalidation, drives watch,
  and lists tasks by parsing instead of executing them.
- **Cheap isolation.** V8 isolates and contexts are cheap in one process,
  so gen files can run in parallel, each confined, with no process spawn,
  and trust tiers (project, ecosystem, third party) could get their own global object
  (per-tier contexts, an opt-in on the roadmap's future list).
- **Polyglot sandboxing.** V8 runs WASM, and a WASM module imported as an ES module
  declares its capabilities by its imports, the same thing a JS module's imports are
  (one instantiated from bytes at run time is a declared entry, like a computed import).
  So one policy file, one inference, and one kernel backstop cover every language that targets WASM,
  and a tool, a plugin, or an agent's script written in Rust, C, Go, Zig, or Python
  runs under the grants its imports declare, beside the TS that calls it, in one process.
  The WASI shape maps onto it directly, with the grant set as the host side of the module's world
  and read roots as its preopens.
  This is where code treated as hostile can go instead of under a looser grant,
  which is what lets the runtime's scope grow past JS without widening the boundary.

## Open questions

Security is the north star, and neither it nor speed is finished.
The design doesn't have every answer, and these are the ones it knows it's missing:

- **Keeping an agent out of the policy file.** Review, harness rules, and a push gate cover the
  normal case, and a signed acceptance that needs a person's presence is designed, built only if
  the measured widening rate makes its friction worth it.
- **Replay needs deterministic scheduling, for gen.** `gro eval` only loads modules, which ESM orders,
  but a hermetic gen run's recorded reads complete on the event loop in no fixed order,
  so it can resolve promises differently between runs.
  The host has to order completions for "replays exactly" to hold by construction,
  and the cost is unmeasured.
- **Scoping per package.** One realm has one global object and one set of shared prototypes,
  so a globals grant reaches every module, and a same-realm dependency can patch an intrinsic
  to intercept a holder's calls and the capabilities passing through them.
  A V8 context per package or trust tier, with frozen intrinsics, would scope both
  (the shape [LavaMoat](https://github.com/LavaMoat/LavaMoat) builds on SES compartments),
  at the cost of realm identity (`instanceof` across contexts), memory,
  and packages that break under it.
  Separate realms are worth exploring as an opt-in tier, but won't be the default,
  since their startup, memory, and compatibility costs weigh against speed and ergonomics.
  How to scope globals within one realm (or whether to), whether to freeze the intrinsics alone,
  and what the opt-in tier looks like are undecided, the limit is stated in the meantime,
  and the gate suite carries a probe either way.
- **Where the builtins' grants live.** `gro check` spawns Node tools through gro's own tasks,
  so "nothing to declare" holds only once the builtins' grants ship with the binary
  and a project's policy file holds project tasks alone.

## Roadmap

The first target is retiring Deno from the Fuz stack before its end of life
in October 2027 (see [Why](#why)).
Until Node leaves much later, third-party Node tools (Vite, vitest, eslint, svelte-check)
run as Node subprocesses under the invocation's kernel rules.

- [x] Rust workspace and a benchmark harness that times gro beside Node, Deno, and Bun
      on the same workloads, the baseline every milestone is measured against
- [x] `gro run` (as `fgro run` for now), running ESM from disk on V8 with web-standard globals
      and a startup snapshot
- [x] TypeScript by tsv's type strip, in process
- [x] strict package resolution, through `node_modules` by `"exports"` up to the project root,
      with `"type": "module"` (`#` imports come later)
- [x] origin containment in the resolver ([resolver](resolver.md#departures-from-node)),
      keeping every module inside its project's or package's root,
      and the loader reading only the file the resolver found
- [ ] `gro eval`, a confined, deterministic one-shot eval of a TS module to JSON
      under the kernel backstop, for any tool that evaluates config;
      it replaces Deno for config eval in the Fuz stack
- [ ] the first release, `fgro` 0.1.0 for Linux x86_64 as a GitHub Release asset
      with `run` and `eval`, a SHA-512 checksum file, and a provenance attestation;
      the tool that installs it records the binary's hash as a pin,
      which the caller of `gro eval` verifies before every spawn
- [ ] the security gate, a suite of escape probes run under every grant set,
      with every web global and member classified by the authority it carries as the oracle,
      grown from `gro eval`'s acceptance tests and passed by every milestone after,
      with the type strip fuzzed over real packages, since it runs on `node_modules`
- [x] on the Node gro, `gro task <path>` replaces the bare `gro <name>` task search
      and the loader is strict, so the scripts the Deno exit ports are written once, in the final shape
- [ ] on the Node gro, `gro check` infers each task's grants, records each package's use of the authority globals,
      and diffs both against the committed policy file, labeled advisory,
      since Node enforces none of it
- [ ] the Deno exit, with config eval on `gro eval` and the stack's other Deno uses on Node
- [ ] startup: a tuned snapshot, a content-keyed code cache, lazy host init
- [ ] gro on gro, with tasks, gen, and check running on the runtime and enforcing the policy file
      (until then the CLI passes every host module, and the network stays declared)
- [ ] the Svelte compiler in-engine
- [ ] macOS and Windows builds, each with its own sandbox design
- [ ] binary-only, where `fgro` becomes `gro`, the npm package carries types, and the crates publish
- [ ] bundled dev and build on the same host, and Vite leaves the loop
- [ ] serving, a test runner, and types from the binary,
      so fuz_app's server side runs on gro and Node leaves the stack's toolchain

Future (unknown order):

- FFI as a grant, in its own confined process
- the `fixed` and `seeded` determinism modes for tasks and gen (`gro eval` ships with `deny`),
  and gen as a memoized function of its graph
- owning the core, replacing deno_core's pieces one at a time, measured
- `fetch`, `crypto.subtle`, and `WebSocket` as consumers need them
- size: filtered ICU data, V8 build flags
- per-package grants for the authority globals, enforced
- V8 contexts per package or trust tier, as an opt-in tier, never the default

Maybe later or never:

- a custom V8 build
