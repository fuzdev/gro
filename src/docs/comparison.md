# comparison

> How Gro's runtime compares to Node, Deno, and Bun:
> the design differences in prose, then the same ground as tables by axis,
> with the measured startup and size

> **status**: Gro's cells tell built from designed.
> The incumbents' cells describe Node 24, Deno 2, and Bun 1 as of October 2026,
> from their documentation and the versions the [bench](#measured) ran;
> a cell marked `?` was reasoned from the runtime's design rather than checked against its docs.
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
Gro starts there and takes the other side of most of Deno's later choices:

- **Node:** Deno added `node:*`, `npm:`, and CommonJS; gro declines all three.
  Not because the compat layer skips checks (Deno runs npm packages and CommonJS under its permission system)
  but because `process`, available everywhere, `require`, and CommonJS
  make authority reachable without a visible import.
- **Imports:** Deno resolves `https:`, `jsr:`, and `npm:` specifiers, fetching code at run time
  (a static remote import from Deno's default hosts needs no permission, other hosts take `--allow-import`;
  `--no-remote` and `--cached-only` turn remote loading off);
  gro reads files on disk, resolved strictly, so the graph is whatever the lockfile installed.
- **Namespace:** Deno has `Deno.*`; gro has no runtime namespace.
- **TypeScript:** Deno transpiles the full language;
  gro strips erasable syntax only, so positions never move.
- **Native code:** Deno's FFI runs in-process under `--allow-ffi`;
  gro's planned FFI runs in a confined process of its own, never the runtime's.

Each of these is a guarantee by omission.
Deno can offer an opt-out, and does for some,
but can't remove the path without breaking its users.
Gro commits to never adding one.
Where gro keeps a Node convention, like `exports` resolution through `node_modules`,
it's because that's the global maximum for gro across security, speed, and ergonomics,
with security first, not compatibility for its own sake.

One difference is structural, and the rest of the sandbox follows from it.
In Deno, authority is reached through globals (`Deno.readFile`, `Deno.Command`, `fetch`):
a permission belongs to the thread, so every module on it holds it
(Deno's docs say as much: modules on one thread can't hold different privileges),
and what a program can reach is a dynamic fact, known by running it.
In gro, authority enters by import, and there's no namespace to reach around the loader.
Deno can't adopt this as its model without breaking every program that calls a `Deno.*` function;
as an opt-in it would be a second runtime with its own standard library.
What follows from it:

- **Who holds a grant.** A gro grant belongs to the module graph that imports the host module,
  and a dependency's reach is attributed to the import chain that brought it in,
  so the diff says which package newly needs the network, not only that something does.
  A capability handed down by reference stays attributed to the module that imported it;
  handing never widens.
- **Where it fails.** Deno denies at the call, deep inside a dependency,
  with a `NotCapable` error the code can catch and swallow.
  Gro denies before the code runs: an ungranted host module doesn't link,
  an undeclared `fetch` isn't there.
  What's left for the call is the scoped check inside a grant
  (a path outside the root, a host not accepted), and the host reports those too.
- **Day to day.** A Deno run carries its flags on every invocation
  (`--allow-net=…`, `--allow-read=…`), bakes them into a task, or takes `-A`.
  A gro run carries nothing: the grants come from the import graph,
  and the policy file changes only when the graph newly reaches authority,
  which is lockfile cadence, not run cadence.
  Inference is sound because every entry point is an import;
  no sound equivalent exists over `Deno.*` calls, which any expression can reach.
- **WASM.** A WASM module imported as an ES module declares its capabilities by its imports too,
  so one policy file and one inference cover both.

In one line: Deno's lockfile pins which code runs;
gro's policy file also pins what that code can reach,
and treats the second lock like the first: committed, diffed, reviewed.

The one corner of gro's model shaped like Deno's is the authority globals.
`fetch` and `WebSocket` reach every module in the graph and can't be inferred,
so they're declared, absent until granted, and shown as the wider class in the policy diff;
the host's `net` import is the narrow way to the same thing, and the hint for a free `fetch` names it first.

The rest are defaults: choices Deno could adopt without breaking anyone,
some of which it already offers behind a flag.

|                   | Deno                                                                     | gro                                                                                                                                                                                                                        |
| ----------------- | ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| prompts           | asks when stdout is a terminal, or `--no-prompt`                         | never asks and has no dynamic decision authority: the source proposes, the diff is accepted in review, the pending run says so, and no hint suggests `-A`                                                                  |
| denials           | a thrown error the code can catch                                        | reported by the host, so a `try/catch` can't hide one; under `gro check` and CI a denial fails the run                                                                                                                     |
| children          | `--allow-run` spawns outside the sandbox                                 | the grant set becomes kernel rules for every child, coarser than in-process (paths and ports, not hosts): an argv, never a shell string, executables resolved when granted, an environment that is empty plus an allowlist |
| kernel            | the op checks stand alone                                                | Landlock and seccomp under the op checks, so an op-check bug or an engine escape still lands in a box                                                                                                                      |
| denied regardless | `--deny-read=…`, opted into per run                                      | inside a granted root, `.env*`, credential files, and writes under `.git` and `node_modules` stay denied short of `-A`; credentials reach one process as named grants, never file reads                                    |
| determinism       | `--seed` fixes randomness; the clock, time zone, and locale have no flag | a hermetic run fixes or denies clock and randomness and replays exactly                                                                                                                                                    |
| eval              | `--v8-flags=--jitless` and a heap limit, by flag                         | `gro eval`: its own process, the JIT off unless a flag turns it on, no code generation from strings, a heap cap, and no environment variable configures it, every knob a flag                                              |

Deno has pieces gro doesn't: workers with a reduced permission set,
a runtime API to query, request, and revoke permissions, `--deny-*` overrides,
the permission broker, `deno compile`, a package manager, formatter, linter, and test runner in the binary,
and years of production hardening.
Gro's claim is the shape of the boundary, not its maturity.

## Compared to Node

Node is the reference the others are measured against, and the host of the tools gro still wraps
(Vite, vitest, eslint, svelte-check) until each is replaced.
Its API hands every module ambient authority: `process`, `require`, and `node:fs`
are reachable from anywhere, which is what makes a compat layer incompatible with gro's model.
Node's permission model (`--permission`, with `--allow-fs-read`, `--allow-fs-write`,
`--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-wasi`)
is opt-in, process-wide, and leaves the network out, so a run under it still fetches anywhere.
What gro keeps from Node is the ESM half:
mandatory extensions on relative imports, `exports` with conditions, `node_modules` as the package source,
and type stripping of erasable syntax only (Node's rule since 22.18, so TypeScript written for Node's stripper runs on gro's).
What gro drops is the legacy half: CommonJS, `main`, extensionless probing, and the format sniff.

## Compared to Bun

Bun is speed-first: JavaScriptCore, its own transpiler, a bundler, a test runner, and a package manager in one binary,
with the broadest Node compatibility after Node itself.
It has no permission model, so a run holds everything,
and its loader probes extensions and index files, a convenience gro refuses.
On the bench Bun's startup is the floor gro's startup milestone chases, about half of what V8's own initialization costs;
its steady-state throughput differs from the V8 runtimes' because the engine does, in both directions.
Gro doesn't compete with Bun on compatibility or batteries;
the comparison is what a run can reach, where Bun has nothing to compare.

## By axis

Gro's cells use four words.
**built** is in `fgro` today; **designed** is in the public [design](runtime.md#design) and the roadmap, not built;
**later** is on the roadmap's future list with no design yet; **no** is declined by design.

### Engine and host

|                   | Node                | Deno                                                       | Bun               | Gro                            |
| ----------------- | ------------------- | ---------------------------------------------------------- | ----------------- | ------------------------------ |
| engine            | V8                  | V8                                                         | JavaScriptCore    | V8                             |
| host language     | C++                 | Rust                                                       | Zig and C++       | Rust                           |
| license           | MIT                 | MIT                                                        | MIT               | 0BSD                           |
| steward           | OpenJS Foundation   | Cloudflare, maintenance only, end of life October 2027     | Oven              | fuz                            |
| runtime namespace | `process`, `node:*` | `Deno`                                                     | `Bun`, `bun:*`    | none                           |
| startup snapshot  | user-land, by flag  | built in                                                   | built in          | built                          |
| platforms         | Linux, macOS, Windows, and more | Linux, macOS, Windows                          | Linux, macOS, Windows | Linux x86_64; macOS and Windows designed |

### Modules

|                                      | Node                        | Deno                                                                       | Bun                  | Gro                                                  |
| ------------------------------------ | --------------------------- | -------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------- |
| ES modules                           | yes                         | yes                                                                        | yes                  | built                                                |
| CommonJS                             | yes                         | yes, through npm compat                                                    | yes                  | no                                                   |
| mandatory extensions, relative imports | yes in ESM                | yes                                                                        | no, probes           | built                                                |
| `exports` and conditions             | yes                         | yes                                                                        | yes                  | built: `gro`, `import`, `default`, in package key order |
| `main` for a package without `exports` | yes                       | yes                                                                        | yes                  | no, refused with the fix                             |
| `#` subpath imports                  | yes                         | yes                                                                        | yes                  | designed                                             |
| import maps                          | no, `imports` only          | yes                                                                        | no                   | no, deferred                                         |
| remote `https:` imports              | no                          | yes: default hosts without a flag, other hosts under `--allow-import`      | no                   | no                                                   |
| `npm:` and `jsr:` specifiers         | no                          | yes                                                                        | `?`                  | no                                                   |
| package source                       | `node_modules`              | its cache, or `node_modules`                                               | `node_modules`       | `node_modules`, walked by `exports` only             |
| search bound                         | the filesystem root         | the filesystem root                                                        | the filesystem root  | built: the project root, failing closed              |
| format of a `.js` file               | `package.json` `type`, else sniffed | `package.json` `type`                                               | `package.json` `type`, else sniffed | built: `package.json` `type`, else refused |
| a dependency's reach is attributable | no                          | no                                                                         | no                   | designed: by the import chain                        |

### TypeScript

|                                 | Node                                                       | Deno                           | Bun                   | Gro                                              |
| ------------------------------- | ---------------------------------------------------------- | ------------------------------ | --------------------- | ------------------------------------------------ |
| runs `.ts` with no build step   | yes, since 22.18                                           | yes                            | yes                   | built                                            |
| transform                       | strip erasable syntax; the rest under `--experimental-transform-types` | full transpile       | full transpile        | built: strip erasable syntax only, by tsv        |
| positions preserved, no source map | yes                                                     | no, source maps                | no, source maps       | built                                            |
| refused syntax                  | `enum`, namespaces, parameter properties, without the flag | none                           | none                  | built: the same set, each with a fix ([typescript](typescript.md)) |
| `.ts` inside `node_modules`     | refused                                                    | `?`                            | yes                   | built: stripped like any other                   |
| type checking                   | no                                                         | `deno check`, not on run       | no                    | no; `gro typecheck` runs tsc or svelte-check     |
| JSX in `.tsx`                   | no                                                         | yes                            | yes                   | no                                               |
| `tsconfig.json` read at run time | no                                                        | `compilerOptions` in `deno.json` | yes, for paths and JSX | no                                             |

### Authority

|                                 | Node                                               | Deno                                                                | Bun     | Gro                                                                                         |
| ------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------- |
| default                         | everything                                         | nothing; flags grant                                                | everything | designed: confined, the import graph's reach plus the policy file                        |
| opt-in model                    | `--permission`                                     | `--allow-*` and `--deny-*`, with allowlists                         | none    | `-A` is the escape hatch, never a default                                                   |
| unit of a grant                 | the process                                        | the process (every module on a thread holds it)                     | —       | designed: the module graph, attributed to the importing chain                               |
| how a grant is declared         | flags                                              | flags, or baked into a task                                         | —       | designed: host modules inferred from imports, authority globals declared, the policy file authorizes |
| where a denial lands            | at the call, `ERR_ACCESS_DENIED`                   | at the call, `NotCapable`, catchable                                | —       | designed: at link time for an ungranted module; scoped denials reported by the host         |
| prompts                         | no                                                 | yes on a terminal, `--no-prompt` off                                | —       | never                                                                                       |
| network scoping                 | not covered                                        | by host and port                                                    | —       | designed: by host; a blanket grant accepted, never inferred                                 |
| child processes                 | `--allow-child-process`, unconfined                | `--allow-run` with an executable allowlist, outside the sandbox     | ambient | designed: under the invocation's kernel rules (Linux)                                       |
| kernel backstop                 | none                                               | none                                                                | none    | designed: Landlock and seccomp (Linux)                                                      |
| native code in-process          | N-API addons, `--allow-addons`                     | FFI under `--allow-ffi`, and N-API                                  | FFI (`bun:ffi`), and N-API | never; FFI later, as a grant in its own process                                  |
| code fetched at run time        | no                                                 | yes, imports                                                        | no      | no                                                                                          |
| determinism                     | no                                                 | `--seed` for randomness                                             | no      | designed: hermetic runs fix or deny clock and randomness                                    |
| authority recorded with the code | no                                                | no                                                                  | no      | designed: the policy file beside the lockfile                                               |
| a sandboxed eval mode           | no                                                 | composed from flags (`--v8-flags=--jitless`, a heap limit)          | no      | designed: `gro eval`, its own process, JIT off by default, no code from strings, a heap cap  |
| workers with fewer permissions  | no                                                 | yes, unstable                                                       | no      | later, if workers land                                                                      |

### Web platform and Node compatibility

|                                            | Node                    | Deno                        | Bun                     | Gro                                             |
| ------------------------------------------ | ----------------------- | --------------------------- | ----------------------- | ----------------------------------------------- |
| `fetch`                                    | yes                     | yes                         | yes                     | later, as a declared grant                      |
| `WebSocket`                                | yes                     | yes                         | yes                     | later                                           |
| streams, `URL`, encoding, timers, `AbortController`, `structuredClone`, `Blob` | yes | yes                 | yes                     | built                                           |
| `crypto.subtle`                            | yes                     | yes                         | yes                     | later                                           |
| WinterTC minimum common API                | a member runtime        | a member runtime            | a member runtime        | the conformance target, graded by its WPT subset under a full grant set |
| `node:*` built-ins                         | native                  | yes, a compatibility layer  | yes, most               | no                                              |
| npm packages                               | yes                     | yes                         | yes                     | built: ES modules with `exports` only           |
| workers                                    | `worker_threads`        | Web Workers                 | Web Workers             | later, maybe                                    |
| HTTP server                                | `node:http`             | `Deno.serve`                | `Bun.serve`             | later, at the Node exit                         |
| WebAssembly                                | yes                     | yes                         | yes                     | built, under the same grants when they land     |

### Toolchain

|                        | Node                              | Deno                          | Bun                   | Gro                                              |
| ---------------------- | --------------------------------- | ----------------------------- | --------------------- | ------------------------------------------------ |
| package manager        | npm, separate                     | built in                      | built in              | npm; fuz's later                                 |
| test runner            | `node:test`                       | `deno test`                   | `bun test`            | later; vitest through `gro test` until then      |
| formatter and linter   | none                              | `deno fmt`, `deno lint`       | none                  | `gro format` on tsv; lint later                  |
| bundler                | none                              | `deno bundle`                 | `bun build`           | later, with bundled dev and build                |
| single-file executable | SEA, experimental                 | `deno compile`                | `bun build --compile` | no                                               |
| task runner            | npm scripts                       | `deno task`                   | `bun run`             | `gro task <path>`, builtins as subcommands       |
| TypeScript types for the host API | `@types/node`          | from the binary               | `bun-types`           | later, from the binary; the npm package until then |

## Measured

From the [bench](runtime.md#design) (suite `baseline` v3, 2026-10-17, one quiet machine, interleaved, medians of 200 samples;
Node 24.14.1, Deno 2.8.3, Bun 1.4.3; `fgro` from its startup snapshot, before any host module).
Node's row has its compile cache on, its best configuration; the W1 module-graph rows wait for `fgro` on TypeScript.

|                              | Bun  | fgro | Node, compile cache | Deno  |
| ---------------------------- | ---- | ---- | ------------------- | ----- |
| empty script, ms             | 3.2  | 6.6  | 17.5                | 20.7  |
| `console.log`, ms            | 3.3  | 6.8  | 18.9                | 20.8  |
| one small import, ms         | 3.8  | 6.7  | 17.7                | 20.8  |
| binary, MB                   | 89.0 | 55.8 | 122.5               | 107.9 |
| binary, gzipped MB           | 42.0 | 20.2 | 43.6                | 43.8  |

Bun's startup is JavaScriptCore's; the V8 runtimes share an engine floor of a few milliseconds that only a custom V8 build or process reuse reaches.
The numbers move with every milestone and are re-recorded with the suite's version named, never compared across versions.
