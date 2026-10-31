# glossary

> The terms the runtime docs use, each defined once

> **status**: most of these describe the design, not the build.
> [run](run.md) covers what's built, and [runtime](runtime.md) marks the rest on its roadmap.

## access manifest

The record of what a run actually used (files read, processes spawned, env read, hosts reached),
taken by the host as each call happens and attributed to the module that made the call.
It's evidence of [use](#reach-need-and-use), and it enforces nothing.

## authority globals

The web globals that reach the world, like `fetch`, `WebSocket`, `EventSource`, and web storage.
A global isn't an import edge, so they can't be soundly [inferred](#inference),
and they're [declared](#declared) instead, absent until granted.
One [realm](#realm) has one global object, so a grant of one covers every module in the run.
Compare [pure globals](#pure-globals) and [host modules](#host-modules).

## composition root

The module where capabilities enter a program and get handed down,
like a task file, an entry module, or a server's main.
It's where host modules are best imported, so the policy diff points at a few files.

## confined

The default [run kind](#run-kinds), bounded by its grants and not replayable.

## declared

A grant a person writes, in a static `grants` literal or a flag, rather than one [inferred](#inference).
Authority globals are always declared. A declaration is a proposal until the [policy file](#policy-file) accepts it.

## escape hatch

`-A` (`--allow-all`), which runs a run [unconfined](#unconfined).
It's the one blanket escape hatch, never a default.
Narrower ways to widen sit beside each default, like the globals grant and the JIT flag for `gro eval`.

## grant

One accepted reach, like a host module, named network hosts, a read root, or an authority global.
A grant is always of something specific, so even a wide one keeps a run [confined](#confined).

## handle

A capability as a value, like the `net` handle, a directory handle, or a spawn handle for one executable.
Handles are [narrowed](#narrowing) and passed down like any other argument.

## handing

Passing a [handle](#handle) to other code.
Handing never widens what the policy sees, since a handed capability stays attributed to the module that imported it.
A grant limits what code can obtain, never what it's given.

## hermetic

The strictest [run kind](#run-kinds), with no network or spawn,
clock and randomness fixed or denied, and every read recorded,
so the run replays exactly and its output can be cached.

## host modules

The imports that reach the OS (fs, process, spawn, watch, net), from [`fuz_host`](https://github.com/fuzdev/fuz_host).
Each is a visible edge in the import graph, so a run's host modules are [inferred](#inference).

## inference

Computing a run's grants from its import graph, before any code runs.
It's sound for host modules, since every way to reach them is an import,
and not possible soundly for authority globals, which any expression can name.

## interposition

Patching a shared prototype (`Array.prototype.push`, `Promise.prototype.then`)
to see the calls and values that pass through it,
including capabilities the patching code was never handed.
It's why one [realm](#realm) isn't a boundary against a dependency built to steal.

## kernel backstop

On Linux, the grant set enforced again as kernel rules (Landlock and seccomp),
installed before V8 starts and inherited by every child process.
It's coarser than the runtime's own checks (paths and ports, not single files or hostnames),
so it backs them up rather than replacing them.

## narrowing

Making a [handle](#handle) reach less before handing it down,
like fewer hosts, one subdirectory, or read-only access.

## origin

Where the resolver says a module came from, the project or a named package:
its [root](#root)'s, a function of the module's real path, never of who imported it.
It decides which package a reach is attributed to and which grants need acceptance.
[resolver](resolver.md) has the rules.

## origin containment

Every module stays inside its [root](#root), checked on its real path:
a relative or absolute import never leaves the importer's root, enters `node_modules` below it,
or enters another root nested in it, and `"exports"` targets and self-references stay inside their package.
[resolver](resolver.md#departures-from-node) has the rules.

## policy file

The committed record of the grants accepted in review, currently `gro.policy.json`.
It holds each task's and gen file's grants and [run kind](#run-kinds),
and `gro check` diffs it like a lockfile, failing on a widening until a person accepts it.

## project root

The directory of the first `package.json` above a module, outside `node_modules`,
that isn't a [type marker](#type-marker), found below any world-writable directory.
The `node_modules` search ends there, and a module with none gets no bare imports
except a self-reference, which resolves through its own package's `"exports"`
and stays inside its package.
The entry's project root is its graph's first [root](#root).

## pure globals

The web globals that carry no authority, like `URL`, `TextEncoder`, streams, and `console`.
They're always present.

## reach, need, and use

Three views of what code can do, each answered in a different place:

- **reach** is what code could get to, a static fact of the import graph and the policy file,
  seen per root or package, and the only one of the three the runtime enforces
- **need** is what a function asks for, static in its parameter types (`deps: FsDeps`),
  visible to reviewers and the type checker but not to the policy
- **use** is what a run actually did, recorded dynamically in the [access manifest](#access-manifest),
  which is evidence, not enforcement

Importing host modules deep in library code makes reach fine-grained.
Passing them as parameters keeps reach at the roots and puts the detail in need and use.

## realm

One global object and its set of shared prototypes (the intrinsics).
Every module in a realm shares them, and a run has one realm by default.
Separate realms per package or trust tier are being explored as an opt-in, never the default.

## review signal

Something that reports without enforcing, like the [scan](#scan) or the advisory grants on the Node gro.
The docs and the CLI name each one as a signal, with what evades it, never as a boundary.

## root

A directory whose modules share an [origin](#origin): the entry's [project root](#project-root),
or the real directory of a package a bare import admitted.
A module's root is the innermost admitted one above its real path,
and a relative or absolute import resolves only inside its importer's root,
never into `node_modules` below it or into another root nested in it.

## run kinds

Every run is one of three by what it reaches and depends on,
[hermetic](#hermetic), [confined](#confined), or [unconfined](#unconfined),
derived from its grants and recorded in the policy file.

## scan

The planned per-package record of which authority globals a package's code references by name,
so a package that starts calling `fetch` shows in the policy diff.
It's a [review signal](#review-signal). Code that reaches a global indirectly
(`globalThis[name]`, `Reflect.get`, a reference handed in) gets past it,
though dynamic access to the global object is itself flagged.

## security gate

A suite of escape probes run under every grant set,
the way the bench measures speed, and passed by every milestone after `gro eval`.

## type marker

A `package.json` whose keys are all `"type"` or `"sideEffects"` (an empty one too),
like `dist/esm/package.json`. It sets a directory's module format without bounding the project.

## unconfined

The [run kind](#run-kinds) that reaches everything, under `-A` or the named unconfined phases
`publish` and `deploy`, which push and use credentials.
