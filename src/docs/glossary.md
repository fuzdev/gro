# glossary

> The terms the runtime docs use, each defined once

> **status**: most of these describe the design, not the build.
> [run](run.md) covers what's built, [authority](authority.md) states the model, and [runtime](runtime.md) marks the rest on its roadmap.
> `gro eval` names the design; the binary built today runs it as `fgro eval`.

## access manifest

The record of what a run actually used (files read, processes spawned, env read, hosts reached),
taken by the host as each call happens.
A host op isn't told which module called it, and finding out takes a stack walk per call,
so how finely a use is attributed (the entry, a package, or a module) is open.
It's evidence of [use](#reach-need-and-use), and it enforces nothing.

## admission

A [root](#root) joining a graph: the entry's [project root](#project-root) is admitted first,
and a package's real directory when a bare import first resolves into it.
The loader holds the admitted roots; the resolver admits nothing ([resolver](resolver.md#roots-and-origins)).

## authority globals

The web globals that would carry authority. A web API carries authority if it can:

- reach state outside the run, like the network (`fetch`, `WebSocket`, `EventSource`)
  or storage that outlives it (web storage, IndexedDB, the Cache API)
- reach another run or context without being handed a reference,
  by a name or origin any code can supply (`BroadcastChannel`, `SharedWorker`)
- load or start code outside its module graph (`Worker`)

A channel made by reference is pure: both ends of a `MessageChannel` are values someone hands over.
APIs that only observe the machine or the time (the clock, randomness, the locale, hardware details)
don't carry authority; the [run kind](#run-kinds) sets them.

Gro has no authority globals. A global isn't an import edge, so it can't be soundly [inferred](#inference),
and one [realm](#realm) has one global object, so a grant of one would cover every module in the run.
Each is an import instead, if Gro implements it, like the network on the `net` [handle](#handle).
Every other web global is a [pure global](#pure-globals).
Compare [host modules](#host-modules).

## box

The kernel rules (Landlock and seccomp) a process is held in on Linux.
`fgro eval`'s box is built, installed from the process's one thread before V8 starts, after it closes every descriptor it inherited above stderr:
file contents read-only to the [project root](#project-root) and its [link closure](#link-closure)
(a path's metadata stays readable everywhere),
no write, no execution, no network,
and no signal outside it, nor a resource limit, priority, or scheduling set at all.
The first built part of the [kernel backstop](#kernel-backstop) ([eval](eval.md#the-box)).
The box for runs and their children is designed ([backstop](backstop.md#the-box-for-runs-and-children)).

## builtin module

A [host module](#host-modules) the runtime implements itself, named by a bare specifier in the binary's builtin table,
like `@fuzdev/gro/args.ts`, the one on `fgro` so far.
It resolves ahead of package lookup to its `builtin:` URL, which imports nothing when written as is,
and its importer must be [granted](#grant) its host module's grant, when it needs one, checked when the graph links;
`fgro eval` imports none, and a package's modules can't reach one through a project module they import by name.
Not a builtin task, which Gro's CLI runs by name.
[resolver](resolver.md#builtin-modules) has the rules.

## composition root

The module where capabilities enter a program and get handed down,
like a task file, an entry module, or a server's main.
It's where host modules are best imported, since inference proposes a grant for every entry whose graph links the importing module ([host modules](host_modules.md#where-to-import-them)).

## confined

The default [run kind](#run-kinds), bounded by its grants and not replayable.
It's a policy property, not by itself an OS sandbox: the [kernel backstop](#kernel-backstop) is what makes it hold
against code that escapes the engine, and neither is built for `fgro run` yet.

## declared

A grant a person writes, in a static `grants` literal or a flag, rather than one [inferred](#inference):
named hosts, a read root outside the project, a spawn allowlist.
A declaration is a proposal until the [policy file](#policy-file) accepts it.

## determinism modes

How a run sees the clock and randomness: `real` (the default for `run` and tasks), `deny` (every read throws; `fgro eval`'s one mode, built),
and `fixed` and `seeded` (a fixed clock and a seeded generator, designed for hermetic runs that need a time or random values).
[eval](eval.md#determinism) lists what `deny` refuses.

## driver

A module a tool hands `fgro eval` on stdin to drive the evaluated module, rather than reading an export:
it evaluates first, is called with the module's namespace, and returns the value.
It's import-free. [eval](eval.md#the-driver) has the contract.

## envelope

The one JSON object `fgro eval` writes, success or failure:
the value, or an error with its kind, which a caller branches on.
[eval](eval.md#the-envelope) has its fields.

## escape hatch

`-A` (`--allow-all`), which makes a run [unconfined](#unconfined).
It's the one flag that widens everything, never a default; a few grants are as strong (a shell, a child's whole environment) and are shown that way.
Narrower ways to widen sit beside each default, like a blanket net grant and the JIT flag for `gro eval`.

## grant

One accepted reach, like a host module, named network hosts, or a read root.
A grant is always of something specific, so even a wide one keeps a run [confined](#confined).

## graph dump

The module graph an `fgro eval` run loaded, written into its [envelope](#envelope) with `--graph`:
each module and `package.json` it read, by an id relative to the [project root](#project-root) and the hash of the bytes read,
and each path it went past finding nothing there, so a cache can tell when any changes.
[eval](eval.md#the-graph) has its fields.

## handle

A capability as a value, like the `net` handle, a directory handle, or a spawn handle for one executable.
Handles are [narrowed](#narrowing) and passed down like any other argument.

## handing

Passing a [handle](#handle) to other code.
Handing never widens what the policy sees, since a handed capability stays attributed to the origin, the project or a package, whose module imported it.
A grant limits what code can obtain, never what it's given.

## hermetic

The strictest [run kind](#run-kinds), with no network, spawn, storage, or by-name channels,
clock and randomness fixed or denied, and every read recorded,
so the run replays exactly and its output can be cached.
Designed; `fgro eval` is the closest built thing to it, with the gaps [eval](eval.md#the-graph) lists
(it records reads only with `--graph`, and its `import.meta.url` is the real path).

## host modules

The imports that reach the OS (fs, spawn, watch, net, and the process modules), Gro's own modules under `@fuzdev/gro/*`.
Each is a visible edge in the import graph, so a run's host modules are [inferred](#inference).
The running process is one module per kind of authority it carries (`env`, `args`, `exit`, `stdin`, `stdout`, `stderr`, `cwd`, `pid`, `runtime_version`),
each designed as its own grant on `fgro` but the observations, which need none.
[host modules](host_modules.md) is the reference.

## inference

Computing a run's grants from its import graph, before any code runs.
Every way to reach authority is an import, so the walk proposes the whole set,
and the loader enforces it at every link, so an `import()` the walk couldn't see (a computed specifier) is still checked against its importer's grants,
and one in code compiled from a string, which has no importer of its own, is refused ([run](run.md#what-loads)).
Inference proposes; the loader enforces.
What a grant reaches past the defaults, like named hosts or a read root outside the project, is [declared](#declared).
A global would defeat it, since any expression can name one, which is why Gro has no [authority globals](#authority-globals).

## interposition

Patching a shared prototype (`Array.prototype.push`, `Promise.prototype.then`)
to see the calls and values that pass through it,
including capabilities the patching code was never handed.
It's why one [realm](#realm) isn't a boundary against a dependency built to steal.

## jitless

V8 running JS in its interpreter alone, with no JIT and no `WebAssembly`:
`fgro eval`'s default, and the box then refuses new executable mappings ([eval](eval.md#the-engine)).

## kernel backstop

On Linux, a coarsening of the grant set enforced as kernel rules (Landlock and seccomp),
installed before V8 starts and inherited by every child process.
It's coarser than the runtime's own checks (hierarchies and ports, not single files or hostnames),
so it backs them up rather than replacing them.
Built so far as `fgro eval`'s [box](#box); [backstop](backstop.md) has the design for the rest.

## link closure

The packages linked into a project's `node_modules` from outside it, found from the directory structure alone:
each `node_modules` entry (and `@scope` entry) whose real path is a package's directory outside the roots found so far
(holding a `package.json` file of its own, holding no root, and held by no world-writable directory),
then the same in each one's own `node_modules`, until nothing new is found.
With the [project root](#project-root), it's what `fgro eval`'s [box](#box) can read.

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
It holds each entry's grants and [run kind](#run-kinds) (for Gro, each task and gen file),
and the project's check (`gro check`) diffs it like a lockfile, failing on a widening until a person accepts it.
It names a spawn grant's programs, while each one's pin, a real path that differs per machine, is designed to live in a local file that isn't committed.

## project root

The directory of the first `package.json` above a module, outside `node_modules`,
that isn't a [type marker](#type-marker), found below any world-writable directory.
The `node_modules` search ends there, and a module with none gets no bare imports
but a [self-reference](#self-reference) ([resolver](resolver.md#departures-from-node)).
The entry's project root is its graph's first [root](#root).

## pure globals

The web globals that carry no authority, like `URL`, `TextEncoder`, streams, and `console`.
No grant adds or removes one: each is installed, or absent only until a consumer needs it (`crypto`, `Headers`).
Under a hermetic run the ones that reveal the clock or randomness are refused or absent ([eval](eval.md#determinism) lists them).

## reach, need, and use

Three views of what code can do, each answered in a different place:

- **reach** is what code could get to, a static fact of the import graph and the policy file,
  seen per root or package, and the only one of the three the runtime enforces (designed)
- **need** is what a function asks for, static in its parameter types (`deps: FsDeps`),
  visible to reviewers and the type checker but not to the policy
- **use** is what a run actually did, recorded dynamically in the [access manifest](#access-manifest),
  which is evidence, not enforcement

Reach is per [origin](#origin) and entry, never per module:
importing a host module deep in library code proposes its grant for every entry that links the module,
while passing handles as parameters keeps each entry's reach to what its composition root wires and puts the detail in need and use.

## realm

One global object and its set of shared prototypes (the intrinsics).
Every module in a realm shares them, and a run has one realm by default.
Separate realms per package or trust tier are being explored as an opt-in hardening of the intrinsics
against [interposition](#interposition), not a way to grant authority, and never the default.

## review signal

Something that reports without enforcing, like the advisory grants on the Node Gro.
The docs and the CLI name each one as a signal, with what evades it, never as a boundary.

## root

A directory whose modules share an [origin](#origin): the entry's [project root](#project-root),
or the real directory of a package a bare import [admitted](#admission).
A module's root is the innermost admitted one above its real path,
and a relative or absolute import resolves only inside its importer's root,
never into `node_modules` below it or into another root nested in it.

## run kinds

Every run is one of three by what it reaches and depends on,
[hermetic](#hermetic), [confined](#confined), or [unconfined](#unconfined),
derived from its grants and recorded in the policy file.

## security gate

A suite of escape probes run under every grant set,
the way the bench measures speed, to be passed by every milestone after `gro eval`.
Designed; today's probes are `fgro eval`'s acceptance tests.

## self-reference

A bare import of the importer's own package by its name, resolved through that package's `"exports"`
and kept inside it, as Node resolves one ([resolver](resolver.md#specifiers)).

## type marker

A `package.json` whose keys are all `"type"` or `"sideEffects"` (an empty one too),
like `dist/esm/package.json`. It sets a directory's module format without bounding the project.

## unconfined

The [run kind](#run-kinds) under `-A` or a named unconfined phase,
like Gro's `publish` and `deploy`, which push and use credentials:
every grant at its widest, with no global added and native code still out of process.
