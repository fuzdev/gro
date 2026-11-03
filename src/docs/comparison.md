# comparison

> How Gro's runtime compares to Node, Deno, and Bun:
> the design differences in prose, then the same ground as tables by axis,
> with the measured startup and size

> **status**: Gro's cells tell built from designed.
> The incumbents' cells describe Node 24, Deno 2, and Bun 1 as of October 2026,
> checked against their documentation and probed on the versions the [bench](#measured) ran.
> Corrections are welcome in [discussions](https://github.com/fuzdev/gro/discussions).

Gro is the [runtime](runtime.md) this compares; its security model and design are there.
The short version: the four runtimes share most of the ingredients
(V8 or JavaScriptCore, web standards, TypeScript without a build step, ES modules),
and differ on what a run can reach and how that is decided.
Node and Bun hand every module everything.
Deno denies by default and grants per process, by flag.
Gro confines by default and grants per module graph, by import,
with the grants recorded in a file beside the lockfile.

## Compared to Deno

If you know Deno, the ingredients are familiar:
V8, web standards, TypeScript without a build step, and deny-by-default permissions.
The difference is where authority comes from.
In Deno it's ambient: `Deno.*`, and through Node compat `process`, `require`, and `node:*`,
are reachable from any module, and a permission granted to the process covers every module in it.
In gro it enters by import, so what a run can reach is a fact about its code, not its command line.
Deno can't adopt that without breaking its users, and it's why gro is a new runtime rather than a set of Deno flags.
A second set of choices Deno could adopt without breaking anyone, and gro turns them on by default.

There's also a practical reason, apart from the design:
Deno is on bug and security fixes until its end of life in October 2027 ([why](runtime.md#why)),
so even the choices it could adopt aren't likely to land.

### What Deno can't adopt without breaking its users

- **Authority enters by import, and nothing is ambient.**
  Gro has no runtime namespace, no `process` global, no `require`, and no `node:*`.
  Deno added each, and every Deno program and every npm package written for Node depends on one,
  not because Deno's compat layer skips checks (it runs npm packages and CommonJS under its permission system)
  but because each makes authority reachable from any module without a visible import.
  A permission in Deno belongs to the thread, so every module on it holds it
  (Deno's docs say as much: modules on one thread can't hold different privileges),
  and what a program can reach is a dynamic fact, known by running it.
  As an opt-in, gro's model would be a second runtime inside Deno with its own standard library.
- **The module graph is closed and on disk.**
  Deno resolves `https:`, `jsr:`, and `npm:` specifiers, fetching code at run time
  (a static remote import from Deno's default hosts needs no permission, other hosts take `--allow-import`;
  `--no-remote` turns remote loading off, and `--cached-only` requires remote modules to be cached already).
  Gro reads files on disk, resolved strictly, so the graph is whatever the lockfile installed,
  and every module stays inside its own project's or package's root.
  Deno offers parts of this as opt-outs, and can't make them the default
  without breaking the programs that import by URL or specifier.

The difference between a guarantee by omission and an opt-out flag is where the guarantee lives.
In Deno, safety belongs to the invocation,
so auditing a project means auditing every way it's run and the flags each one passes.
In gro it belongs to the repo: it holds for every run,
so a reviewer reads the code and the policy file and nothing else.
Gro commits to never adding these paths.
Where it keeps a Node convention, like `exports` resolution through `node_modules`,
it's because that's the global maximum for gro across security, speed, and ergonomics,
with security first, not compatibility for its own sake.

Most of gro's sandbox follows from the first of these:

- **Who holds a grant.** A gro grant belongs to the module graph that imports the host module,
  and a dependency's reach is attributed to the import chain that brought it in,
  so the diff says which package newly imports the network, not only that something does
  (a call to the global `fetch` is the exception below).
  A capability handed down by reference stays attributed to the module that imported it;
  handing never widens.
- **Where it fails.** Deno denies at the call, deep inside a dependency,
  with a `NotCapable` error the code can catch and swallow.
  Gro denies before the code runs: an ungranted host module doesn't link,
  an undeclared `fetch` isn't there.
  What's left for the call is the scoped check inside a grant
  (a path outside the root, a host not accepted), and the host reports those too.
- **Day to day.** A Deno run carries its flags on every invocation
  (`--allow-net=…`, `--allow-read=…`), bakes them into a task,
  loads a named permission set from `deno.json` (`-P`, since 2.5), or takes `-A`.
  A gro run in a project carries nothing: the grants come from the import graph,
  and the policy file changes only when the graph newly imports authority or declares a global,
  which is lockfile cadence, not run cadence.
  Outside a project, or for one run, gro takes Deno's `--allow-*` flags, for that run only.
- **Inference.** It's sound because every entry point but the declared globals is an import,
  so the policy file can be generated from the code and reviewed;
  no sound equivalent exists over `Deno.*` calls, which any expression can reach,
  so Deno's permission set is written by hand.
- **WASM.** A WASM module imported as an ES module declares its capabilities by its imports too,
  so one policy file and one inference cover both.

In one line, Deno's lockfile pins which code runs, and its config can hold a hand-written permission set for the process,
while gro's policy file pins what that code can reach, per import chain for host modules
and per run for the globals, inferred from the graph where it can be,
and is treated like the lockfile, committed, diffed, and reviewed.

Three limits belong beside that claim.
The authority globals are the one corner of gro's model shaped like Deno's:
`fetch` and `WebSocket` reach every module in the graph and can't be soundly inferred,
so they're declared, absent until granted, and shown as the wider class in the policy diff,
and the host's `net` import is the narrow way to the same thing, which the hint for a free `fetch` names first
([imports or globals](runtime.md#imports-or-globals) weighs the two).
The import boundary holds against code that overreaches, not a dependency built to steal,
which can patch a shared prototype in the same realm
(the runtime's [threat model](runtime.md#threat-model) says where the bound is then).
And of all this, the omissions are built (`fgro` has no namespace, no Node compat, and no remote imports),
while the grants by import are designed, not built.

### What Deno could adopt, and gro turns on

These are defaults, choices Deno could adopt without breaking anyone,
some of which it already offers behind a flag.
They matter, but they aren't why gro exists.

|                   | Deno                                                                     | gro                                                                                                                                                                                                                        |
| ----------------- | ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| prompts           | asks on a terminal, unless `--no-prompt`                                 | never asks and has no dynamic decision authority: the source proposes, the diff is accepted in review, the pending run says so, and no hint suggests `-A`                                                                  |
| denials           | a thrown error the code can catch                                        | reported by the host, so a `try/catch` can't hide one; under the project's check and CI a denial fails the run                                                                                                             |
| permission file   | a hand-written permission set in `deno.json` (`-P`)                      | the policy file, committed beside the lockfile and diffed in review, a widening failing the check until accepted; inferred from the imports, which Deno can't do (above)                                                   |
| children          | `--allow-run` spawns outside the sandbox                                 | the grant set becomes kernel rules for every child, coarser than in-process (paths and ports, not hosts): an argv, never a shell string, executables resolved when granted, an environment that is empty plus an allowlist |
| kernel            | the op checks stand alone                                                | Landlock and seccomp under the op checks, so an op-check bug or an engine escape still lands in a box                                                                                                                      |
| denied regardless | `--deny-read=…`, opted into per run                                      | inside a granted root, `.env*`, credential files, and writes under `.git` and `node_modules` stay denied short of `-A`; credentials reach one process as named grants, never file reads                                    |
| native code       | FFI in-process under `--allow-ffi`, which `-A` includes                  | FFI planned as a grant, with the library in its own confined process and no part of `-A`; in-process loading isn't currently planned (feedback welcome in [discussions](https://github.com/fuzdev/gro/discussions)), and if a measured need brings it, it's a grant of its own, shown as leaving the runtime's checks |
| determinism       | `--seed` fixes randomness; the clock, time zone, and locale have no flag | a hermetic run fixes or denies clock and randomness and replays exactly                                                                                                                                                    |
| eval              | `--v8-flags=--jitless` and a heap limit, by flag                         | `gro eval`: its own process, the JIT off unless a flag turns it on, no code generation from strings, a heap cap and a memory cap, and no environment variable configures it, every knob a flag                             |

TypeScript is a simplicity choice rather than a security one:
Deno transpiles the full language, and gro strips erasable syntax only, so positions never move
([typescript](typescript.md)).

### What Deno has that gro doesn't

Workers with a reduced permission set,
a runtime API to query, request, and revoke permissions, `--deny-*` overrides,
the permission broker, `DENO_AUDIT_PERMISSIONS` (a log of what a run reached),
`deno compile`, a package manager, formatter, linter, and test runner in the binary,
and years of production hardening.
Gro's claim is the shape of the boundary, not its maturity.

### Moving a script from Deno

What changes for a script that runs on Deno today, as designed, except where a cell says built:

|                      | Deno                                                                                   | gro                                                                                                                                  |
| -------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| running it           | `deno run --allow-read=. --allow-net=api.example.com main.ts`, or the flags in a task | in a project, `gro run main.ts`, with the grants in the policy file; outside one, the same `--allow-*` flags, for that run only      |
| a missing permission | a prompt on a terminal, else `NotCapable` at the call, which the code can catch        | the run fails before any code runs, naming the grant and the fix, never a prompt                                                     |
| changing permissions | edit the flags, or the permission set in `deno.json`                                   | inferred from the imports; a widening fails the project's check until accepted, like a lockfile change                               |
| host APIs            | `Deno.readTextFile`, `Deno.Command`, `Deno.env`, anywhere                               | imports from the host modules, at the entry module, handed down; the code is ported, not run unchanged                               |
| `fetch`              | a global, scoped by `--allow-net`                                                      | absent until declared; `net.fetch` from the host's net import is the narrow way                                                      |
| dependencies         | `npm:`, `jsr:`, and `https:` specifiers, fetched and cached by Deno                    | built: `node_modules` from npm and a `package.json`; a package without `exports`, or one that's CommonJS, is refused with the reason  |
| TypeScript           | the full language, transpiled, `.tsx` included                                         | built: erasable syntax only, so an `enum` or a parameter property is refused with a fix; no `.tsx`, and no `tsconfig.json` read      |
| `node:*` built-ins   | a compatibility layer                                                                  | none; the web globals and the host modules instead                                                                                   |
| tools in the binary  | `deno fmt`, `lint`, `test`, `compile`, and a package manager                           | none yet; `gro format` and `gro lint` on the Node gro, a test runner planned, no `compile`                                           |
| startup              | slower than gro on every row of the bench                                              | built: faster than Deno on every row ([measured](#measured))                                                                         |

The shape of the move: a script that's already strict ESM over npm packages with `exports`,
reaching the OS in a few places, ports by turning those places into imports.
One built on `Deno.*` throughout, CommonJS packages, or the Node compat layer
ports to Node instead, or waits.

## Compared to Node

Node is the reference the others are measured against, and the host of the tools gro still wraps
(Vite, vitest, eslint, svelte-check) until each is replaced.
Its API hands every module ambient authority: `process`, `require`, and `node:fs`
are reachable from anywhere, which is what makes a compat layer incompatible with gro's model.
Node's permission model (`--permission`, with `--allow-fs-read`, `--allow-fs-write`,
`--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-wasi`, `--allow-inspector`)
is opt-in and process-wide, and in Node 24 leaves the network out (Node 25 adds `--allow-net`, in active development),
so a run under it still fetches anywhere.
What gro keeps from Node is the ESM half:
mandatory extensions on relative imports, `exports` with conditions, `node_modules` as the package source,
and type stripping of erasable syntax only (Node's rule since 22.18, so ES-module TypeScript written for Node's stripper runs on gro's,
with one exception gro refuses and Node strips silently: an `as` whose removal regroups the expression,
`a + b as number / c`, which Node's stripper reads as `a + b / c`).
What gro drops is the legacy half: CommonJS, `main`, extensionless probing, and the format sniff.

## Compared to Bun

Bun is speed-first, with JavaScriptCore, its own transpiler, a bundler, a test runner, a type checker, and a package manager in one binary,
with the broadest Node compatibility after Node itself
(Anthropic acquired it in December 2025; it stays open source under MIT).
It has no permission model, so a run holds everything,
and its loader probes extensions and index files, a convenience gro refuses.
On the bench Bun's startup is the floor gro's startup milestone chases;
its steady-state throughput differs from the V8 runtimes' because the engine does, in both directions.
Gro doesn't compete with Bun on compatibility or batteries;
the comparison is what a run can reach, where Bun has nothing to compare.

## By axis

Gro's cells for a feature start with one of these words:
**built** is in `fgro` today, **designed** is in the public [design](runtime.md#design) but not built,
**planned** is a [roadmap](runtime.md#roadmap) milestone with no public design yet,
**later** is wanted but not scheduled, and **no** is declined by design.

### Engine and host

|                   | Node                | Deno                                                       | Bun               | Gro                            |
| ----------------- | ------------------- | ---------------------------------------------------------- | ----------------- | ------------------------------ |
| engine            | V8                  | V8                                                         | JavaScriptCore    | V8                             |
| host language     | C++                 | Rust                                                       | Rust and C++, since 1.4 (Zig before) | Rust                           |
| license           | MIT                 | MIT                                                        | MIT               | 0BSD                           |
| steward           | OpenJS Foundation   | Cloudflare, maintenance only, end of life October 2027     | Anthropic, since December 2025 | fuz                            |
| runtime namespace | `process`, `node:*` | `Deno`                                                     | `Bun`, `bun:*`    | none                           |
| startup snapshot  | built in; user-land snapshots by flag | built in                                 | no heap snapshot  | built                          |
| platforms         | Linux, macOS, Windows, and more | Linux, macOS, Windows                          | Linux, macOS, Windows | Linux x86_64; macOS and Windows designed |

### Modules

|                                      | Node                        | Deno                                                                       | Bun                  | Gro                                                  |
| ------------------------------------ | --------------------------- | -------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------- |
| ES modules                           | yes                         | yes                                                                        | yes                  | built                                                |
| CommonJS                             | yes                         | yes: `.cjs`, `"type": "commonjs"`, and npm packages                        | yes                  | no                                                   |
| mandatory extensions, relative imports | yes in ESM                | yes, unless `--sloppy-imports`                                             | no, probes           | built                                                |
| `exports` and conditions             | yes                         | yes                                                                        | yes                  | built: `gro`, `import`, `default`, in package key order |
| `main` for a package without `exports` | yes                       | yes                                                                        | yes                  | no, refused with the fix                             |
| `#` subpath imports                  | yes                         | yes                                                                        | yes                  | designed                                             |
| JSON modules (`with {type: 'json'}`) | yes                         | yes                                                                        | yes                  | later                                                |
| import maps                          | no, `imports` only          | yes                                                                        | no                   | no, deferred                                         |
| remote `https:` imports              | no                          | yes: default hosts (deno.land, jsr.io, esm.sh, jsdelivr, GitHub raw) without a flag, other hosts under `--allow-import`      | no                   | no                                                   |
| `npm:` and `jsr:` specifiers         | no                          | yes                                                                        | no; its equivalent is auto-install, a bare specifier with a version suffix (`zod@3.0.0`) fetched when no `node_modules` exists | no |
| package source                       | `node_modules`              | its cache, or `node_modules`                                               | `node_modules`       | built: `node_modules`, walked by `exports` only      |
| search bound                         | the filesystem root         | the filesystem root                                                        | the filesystem root  | built: the project's `package.json`, found before any `node_modules`, failing closed; a world-writable directory ends the walk ([resolver](resolver.md)) |
| relative imports stay inside their project or package | no                | no                                                                         | no                   | built: never out of the importer's root, into `node_modules` below it, or into another package's root, checked on the real path; a package's `exports` targets too |
| format of a `.js` file               | `package.json` `type`, else sniffed | `package.json` `type`, else an ES module, never sniffed           | `package.json` `type`, else sniffed | built: `package.json` `type`, else refused |

### TypeScript

|                                 | Node                                                       | Deno                           | Bun                   | Gro                                              |
| ------------------------------- | ---------------------------------------------------------- | ------------------------------ | --------------------- | ------------------------------------------------ |
| runs `.ts` with no build step   | yes, since 22.18                                           | yes                            | yes                   | built                                            |
| transform                       | strip erasable syntax; the rest under `--experimental-transform-types` | full transpile       | full transpile        | built: strip erasable syntax only, by tsv        |
| positions preserved, no source map | yes                                                     | no, source maps                | no, source maps       | built                                            |
| refused syntax                  | `enum`, value namespaces, parameter properties, `import =`, `export =`, `<T>x`, without the flag; an assertion whose removal regroups the expression is stripped silently | none | none | built: the same, plus the regrouping assertion, each with a fix ([typescript](typescript.md)) |
| a `.ts` file's format           | `"type"` and syntax detection, so CommonJS too             | an ES module                   | sniffed               | built: always an ES module                       |
| `.ts` inside `node_modules`     | refused                                                    | refused                        | yes                   | built: stripped like any other                   |
| type checking                   | no                                                         | `deno check`, and on run with `--check` | `bun check` (a port of typescript-go, since 1.4.3), and on run with `--check` | no; `gro typecheck` runs tsc or svelte-check     |
| JSX in `.tsx`                   | no                                                         | yes                            | yes                   | no                                               |
| `tsconfig.json` read at run time | no                                                        | yes: `deno.json` `compilerOptions`, or `tsconfig.json` in a `package.json` project | yes, for paths and JSX | no                       |

### Authority

|                                 | Node                                               | Deno                                                                | Bun     | Gro                                                                                         |
| ------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------- |
| default                         | everything                                         | nothing; flags grant                                                | everything | designed: confined, the import graph's reach plus the policy file                        |
| opt-in model                    | `--permission`, stable since 22.13 and 23.5        | `--allow-*` and `--deny-*`, with allowlists                         | none    | designed: none needed in a project, confined is the default; `--allow-*` for one run, `-A` the escape hatch |
| unit of a grant                 | the process                                        | the process (every module on a thread holds it)                     | —       | designed: the module graph, attributed to the importing chain; the run, for authority globals |
| a dependency's reach is attributable | no                                       | no                                                                  | —       | designed: by the import chain, for host modules                                             |
| how a grant is declared         | flags                                              | flags, or baked into a task                                         | —       | designed: host modules inferred from imports, authority globals declared, the policy file authorizes; flags for one run |
| where a denial lands            | at the call, `ERR_ACCESS_DENIED`                   | at the call, `NotCapable`, catchable                                | —       | designed: at link time for an ungranted module; scoped denials reported by the host         |
| prompts                         | no                                                 | yes on a terminal, `--no-prompt` off                                | —       | designed: never                                                                             |
| network scoping                 | not covered in 24; `--allow-net` since 25          | by host and port                                                    | —       | designed: by host; a blanket grant accepted, never inferred                                 |
| child processes                 | `--allow-child-process`; a `node` child inherits the flags, any other executable is unconfined | `--allow-run` with an executable allowlist, outside the sandbox     | ambient | designed: under the invocation's kernel rules (Linux)                                       |
| kernel backstop                 | none                                               | none                                                                | none    | built for `fgro eval`: Landlock and seccomp (Linux); designed for every run                 |
| native code in-process          | N-API addons, `--allow-addons`                     | FFI under `--allow-ffi`, and N-API                                  | FFI (`bun:ffi`), and N-API | later: FFI as a grant in its own process; in-process loading not currently planned, open to feedback                                  |
| code fetched at run time        | no                                                 | yes, imports                                                        | no      | no                                                                                          |
| determinism                     | no                                                 | `--seed` for randomness                                             | no      | built for `fgro eval`: clock and randomness denied; designed: hermetic runs fix or deny them |
| authority recorded with the code | no                                                | a hand-written permission set in `deno.json` (`-P`, since 2.5), process-wide    | no | designed: the policy file beside the lockfile, inferred from the graph                 |
| a record of what a run reached  | no                                                 | `DENO_AUDIT_PERMISSIONS`                                            | no      | designed: the access manifest                                                               |
| a sandboxed eval mode           | composed from flags (`--permission`, `--jitless`, `--disallow-code-generation-from-strings`, a heap limit) | composed from flags (`--v8-flags=--jitless`, a heap limit) | no      | built: `fgro eval`, its own process, JIT off by default, no code from strings, a heap cap, a memory cap, a timeout, clock and randomness denied, and a kernel box (Linux): read-only to its project, no exec, no network |
| workers with fewer permissions  | no                                                 | yes, unstable                                                       | no      | later, if workers land                                                                      |

### Web platform and Node compatibility

|                                            | Node                    | Deno                        | Bun                     | Gro                                             |
| ------------------------------------------ | ----------------------- | --------------------------- | ----------------------- | ----------------------------------------------- |
| `fetch`, `Headers`, `Request`, `Response`  | yes                     | yes                         | yes                     | designed, as a declared grant                   |
| `WebSocket`                                | yes                     | yes                         | yes                     | later, as a declared grant                      |
| streams, `URL`, encoding, timers, `AbortController`, `structuredClone`, `Blob` | yes | yes                 | yes                     | built                                           |
| `crypto.subtle`                            | yes                     | yes                         | yes                     | later                                           |
| `crypto.getRandomValues`, `randomUUID`     | yes                     | yes                         | yes                     | designed, on the determinism axis               |
| WinterTC minimum common API                | largely implemented     | largely implemented         | largely implemented     | planned: the conformance target, graded by its WPT subset under a full grant set |
| `node:*` built-ins                         | native                  | yes, a compatibility layer  | yes, most               | no                                              |
| npm packages                               | yes                     | yes                         | yes                     | built: ES modules with `exports` only           |
| workers                                    | `worker_threads`        | Web Workers                 | Web Workers             | later, as a declared grant                      |
| HTTP server                                | `node:http`             | `Deno.serve`                | `Bun.serve`             | planned, at the Node exit                       |
| WebAssembly                                | yes                     | yes                         | yes                     | built: the JS API; `.wasm` imports designed     |

### Toolchain

Gro's cells here name the Node gro's command where the runtime has none yet.

|                        | Node                              | Deno                          | Bun                   | Gro                                              |
| ---------------------- | --------------------------------- | ----------------------------- | --------------------- | ------------------------------------------------ |
| package manager        | npm, separate                     | built in                      | built in              | npm; fuz's later                                 |
| test runner            | `node:test`                       | `deno test`                   | `bun test`            | planned; vitest through `gro test` until then    |
| formatter and linter   | none                              | `deno fmt`, `deno lint`       | none                  | `gro format` on tsv, `gro lint` on ESLint        |
| bundler                | none                              | `deno bundle`                 | `bun build`           | planned, with bundled dev and build              |
| code cache             | opt-in, `NODE_COMPILE_CACHE`      | yes                           | yes                   | designed; never read by `gro eval`               |
| single-file executable | SEA, experimental                 | `deno compile`                | `bun build --compile` | no                                               |
| task runner            | npm scripts, `node --run`         | `deno task`                   | `bun run`             | built on the Node gro: `gro task <path>` with the builtins as subcommands |
| TypeScript types for the host API | `@types/node`          | from the binary               | `bun-types`           | planned, from the binary; the npm package until then |

## Measured

From the [bench](../../crates/gro_bench) (suite `baseline` v5, 2026-11-02, one quiet machine, interleaved, medians of 200 samples;
Node 24.14.1, Deno 2.8.3, Bun 1.4.3; `fgro` from its startup snapshot).
The module graph is fuz_util's `src/lib` modules with only relative imports, loaded as TypeScript each runtime strips itself.
Warm keeps each runtime's on-disk caches; cold turns off their transpile and compiled-code caches.
Node's column has its compile cache on in its warm rows and off in its cold one, so it matters only for the warm graph (plain Node takes 82 ms there);
`fgro` has no on-disk cache, so its cold and warm rows run the same command.

|                              | Bun  | fgro | Node, compile cache | Deno  |
| ---------------------------- | ---- | ---- | ------------------- | ----- |
| empty script, ms             | 3.3  | 7.1  | 18.2                | 21.2  |
| `console.log`, ms            | 3.4  | 7.3  | 19.5                | 21.2  |
| one small import, ms         | 3.9  | 7.3  | 18.5                | 21.3  |
| module graph, warm, ms       | 7.5  | 14.7 | 31.8                | 25.0  |
| module graph, cold, ms       | 8.5  | 14.8 | 81.9                | 29.6  |
| binary, MB                   | 89.0 | 55.3 | 122.5               | 107.9 |
| binary, gzipped MB           | 42.0 | 20.6 | 43.6                | 43.8  |

Confined eval has its own suite (`eval` v1, same day and machine): a zap config written for the bench, in a project with a couple hundred stub packages,
takes 19.0 ms through `fgro eval` and 32.7 ms through `deno run` with the flags zap passed it before moving to `fgro`.

Bun's startup is JavaScriptCore's; the V8 runtimes share an engine floor of a few milliseconds that only a custom V8 build or process reuse reaches.
The numbers move with every milestone and are re-recorded with the suite's version named, never compared across versions.
