# glossary

> The terms the runtime docs use, each defined once

> **status**: most of these describe the design, not the build.
> [run](run.md) covers what's built, [authority](authority.md) states the model, and [runtime](runtime.md) marks the rest on its roadmap.
> `gro eval` names the design; the binary built today runs it as `fgro eval`.

## access manifest

The record of what a run actually used (files read, processes spawned, env read, hosts reached),
taken by the host as each call happens.
A host op isn't told which module called it: attributing a use to a package would take an instance of the host module per origin,
and to a module a stack walk per call, so how finely a use is attributed (the entry, a package, or a module) is open.
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

## boundary

A protection that holds against code built to defeat it, because it's a native check, a separate process, or the kernel,
like the link-time grant check, a [handle](#handle)'s scope, a [pin](#pin), or a [box](#box).
One of the three kinds the docs name each mechanism as, beside a [guardrail](#guardrail) and a [review signal](#review-signal),
each with what defeats it, like an engine exploit for the runtime's own checks ([authority](authority.md#threat-model)).

## box

The kernel rules (Landlock and seccomp, and for a run a network namespace) a process is held in on Linux, with every program it starts:
a coarsening of the grant set, installed from the process's one thread before V8 starts and inherited by every child process,
which backs up the runtime's own checks rather than replacing them.
Built for `fgro eval` ([eval](eval.md#the-box)) and for every command of Gro's the binary runs, each tool it starts in a box of its own inside it
([box](box.md#what-a-commands-box-admits)); designed for `fgro run` ([box](box.md)).

## builtin module

A [host module](#host-modules) the runtime implements itself, named by a bare specifier in the binary's builtin table,
like `@fuzdev/gro/args.ts`; [host modules](host_modules.md#the-modules) lists the ones on `fgro` so far.
It resolves ahead of package lookup to its `builtin:` URL, which imports nothing when written as is,
and its importer must be [granted](#grant) its host module's grant, when it needs one, checked when the graph links,
while one that needs none (`cwd`, `pid`, `runtime_version`, `blake3`, `tsv`, and the error modules) is imported by any module of a run, a package's included;
`fgro eval` imports none, and a package's modules can't reach a granted one through a project module they import by name.
Not a builtin task, which Gro's CLI runs by name.
[resolver](resolver.md#builtin-modules) has the rules.

## command's closure

What one of Gro's commands runs and may reach, read from its command line before any JS runs:
the builtins it runs, each builtin's `invokes` joined in turn as their marks allow
(one marked `when` only when that arg of the command's is on, its default and its `no-` dual applied, so `check --no-test` holds no `test`, and one marked `check` without its writes,
as is the command itself under `--check` when an invoke anywhere marks it `check`, like `gro gen --check`),
and the union of their grants with the CLI's own row,
or why the command runs on the Node Gro instead, like a builtin that declares no grants, an install, or a project task.
The grant flags sit beside it.
The binary reads it natively from the grants table it carries (`gro_runtime::verb`) for `fgro <command>`,
as the [boundary](#boundary) for what grants a run of the command gets, the run held to it joined to the grant flags;
what it can't bound is the union itself, which the command line chose.
Not the [link closure](#link-closure). [task](task.md#a-builtins-host-and-grants) has the marks.

## composition root

The module where capabilities enter a program and get handed down,
like a task file, an entry module, or a server's main.
It's where host modules are best imported, since inference proposes a grant for every entry whose graph links the importing module ([host modules](host_modules.md#where-to-import-them)).

## confined

The default [run kind](#run-kinds), bounded by its grants and not replayable.
It's a policy property, not by itself an OS sandbox: the [box](#box) is what makes it hold
against code that escapes the engine. For `fgro run` and `fgro <command>` its link-time grant check is built,
and the box is built for `fgro <command>` and not yet for `fgro run`.

## declared

A grant a person writes, in a static `grants` literal or a flag, rather than one [inferred](#inference):
named hosts, a read root outside the project, a spawn allowlist.
A declaration is a proposal until the [policy file](#policy-file) accepts it.

## deny overlays

Refusals that hold inside a granted root, the read-deny overlay for reads and the write-deny overlay for writes.
Short of `-A`: reads of `.env*` files except through the host's env reader and of credential files (`~/.ssh`, `.git/config`),
and writes under `.git` and `node_modules`.
Always on, `-A` included: reads of a process's environment, memory, and command line (`/proc/*/environ`, `/proc/*/mem`, `/proc/*/cmdline`) through the runtime's own `fs`;
the [box](#box) a command of Gro's runs in lets its tools read `/proc` and refuses only the environment, memory, and working directory of a process outside it,
and each tool's own box inside it refuses those of every other tool and of the run.
Designed ([authority](authority.md#secure-by-default), [host modules](host_modules.md#native-requirements));
until they land, `fs` on `fgro` reads any file beneath a root that reads (`.env` included)
and writes any beneath a root that writes (`.git` and `node_modules` included) ([host modules](host_modules.md#fs-on-fgro));
in a run of `fgro <command>` the run's [box](#box) refuses those writes at the kernel (outside its [tool directories](#tool-directory) and what a grant flag names), as it refuses writes under every dot-entry, and leaves `.env*` readable.

## determinism modes

How a run sees the clock and randomness: `real` (the default for `run` and tasks), `deny` (every read throws; `fgro eval`'s one mode, built),
and `fixed` and `seeded` (a fixed clock and a seeded generator, designed for hermetic runs that need a time or random values).
[eval](eval.md#determinism) lists what `deny` refuses.
The observation modules (`cwd`, `pid`, `runtime_version`) are on this axis, not the grants':
they need no grant, are real under `fgro run`, and are refused in `fgro eval` with every builtin module;
what a hermetic run does with each is designed, not built ([host modules](host_modules.md#the-process-modules)).

## driver

A module a tool hands `fgro eval` on stdin to drive the evaluated module, rather than reading an export:
it evaluates first, is called with the module's namespace, and returns the value.
It's import-free. [eval](eval.md#the-driver) has the contract.

## envelope

The one JSON object `fgro eval` writes, success or failure:
the value, or an error with its kind, which a caller branches on.
[eval](eval.md#the-envelope) has its fields.

## escape hatch

`-A` (Deno's `--allow-all`, which `fgro run` spells only `-A`), which makes a run [unconfined](#unconfined):
built on `fgro run`, where it grants every builtin module to every module of the run, `env` with every variable, `fs` with every directory, `spawn` with any program.
`fgro <command>` refuses it before the command word, and after it for every command the binary runs itself, since that run is always boxed (a command left to Node passes it to the Node Gro), so a command of Gro's runs unconfined only on the Node Gro (`node_modules/.bin/gro`).
It's the one flag that widens everything but the always-on [deny overlay](#deny-overlays) (designed), never a default; some grants are as strong (any program under `fgro run`, which starts children unboxed, a child's whole environment) and are shown that way.
Narrower ways to widen sit beside each default, like a blanket net grant and the JIT flag for `gro eval`.

## exec root

A directory a run grants for `spawn.program_in` and `find_in`, beneath which they find a program to run: on `fgro`, one `fgro run --allow-run-in=` names, opened as the run starts and held to the directory it named then ([host modules](host_modules.md#spawn-on-fgro)).
One may also be a [write root](#write-root), as a build's output directory is, so a run granted both runs what it writes there, and each is declared and reviewed as narrowly.
In a run of `fgro <command>` the run's box reads and executes beneath each one the grant flags name;
for `fgro run` the kernel's rule on one is designed ([box](box.md#what-a-commands-box-admits)).

## fallback

A command of Gro's that `fgro <command>` runs on the Node Gro the project installed instead of on the binary,
after one `note:` line saying why: its [command's closure](#commands-closure) holds a builtin that declares no grants, an install, or a project task,
the command is `task` listing the project's tasks, the binary leaves a builtin to Node by policy (`clean`), the project installed another version of Gro,
or the kernel's Landlock is missing, off, or below its sixth ABI (Linux 6.12), or a seccomp filter refuses its calls, so the run's [box](#box) can't go up.
Each reason is decided by the command line, by the project's files, or by the host, and `GRO_FALLBACK=command` refuses the last two kinds
([authority](authority.md#what-the-first-release-is-for)).
The line is a [review signal](#review-signal); what runs is the Node Gro, which confines nothing.

## generated entry

The module a run of `fgro <command>` starts from, which the binary generates and holds in memory at `gro:main`:
it imports the host modules the run grants and Gro's `binary_main`, at the project's [origin](#origin) with the working directory as its [project root](#project-root),
and hands Gro's CLI what it imported ([runtime](runtime.md#gros-workflow-on-the-runtime)).

## grant

One accepted reach, like a host module, named network hosts, or a read root.
A grant is always of something specific, so even a wide one keeps a run [confined](#confined).
A `spawn` grant under `fgro run` is the exception until its children are boxed, since a granted program is close to `-A` ([run](run.md#usage));
a command of Gro's starts the programs it's granted in the run's [box](#box).

## graph dump

The module graph an `fgro eval` run loaded, written into its [envelope](#envelope) with `--graph`:
each module and `package.json` it read, by an id relative to the [project root](#project-root) and the hash of the bytes read,
and each path it went past finding nothing there, so a cache can tell when any changes.
[eval](eval.md#the-graph) has its fields.

## Gro's library

The modules of Gro's own `src/lib` that run on `fgro`, which the binary carries, stripped of their types when it's built:
a specifier of `@fuzdev/gro` that isn't a [builtin module](#builtin-module)'s resolves to one, never to a package,
at its `gro:lib/` URL, its [origin](#origin) holding no grant by its own imports,
its packages taken from the project root's `node_modules` at the ranges of Gro's peer dependencies.
[resolver](resolver.md#gros-library) has the rules.

## guardrail

A protection enforced in the shared [realm](#realm) or by convention, which holds against mistakes, not intent,
like a builtin's `host`, a [narrowed](#narrowing) handle in first-party code, or the frozen grants table.
Code built to defeat one does so cheaply, like a project task's `invoke_task` of a builtin or a patched shared prototype,
so the docs never describe one as bounding deliberate code ([authority](authority.md#threat-model)).

## handle

A capability as a value, like the `net` handle, a directory handle, or a spawn handle for one executable.
Handles are [narrowed](#narrowing) and passed down like any other argument.
On `fgro` a directory handle carries a scope, whether it reads and whether it writes, its root's when made and never widened,
and what an op through it may do is decided by that scope, never by the run's roots ([host modules](host_modules.md#fs-on-fgro)).

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
each its own grant on `fgro` but the observations (`cwd`, `pid`, `runtime_version`), which need none.
[host modules](host_modules.md) is the reference, and its [table](host_modules.md#the-modules) marks the ones built on `fgro`.

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

Where the resolver says a module came from: the project, a named package, or [Gro's library](#gros-library).
A module on disk takes its [root](#root)'s, a function of its real path, never of who imported it;
a module of Gro's library has no file, and its origin, Gro's, holds no grant by its own imports.
It decides which package a reach is attributed to and which grants need acceptance.
[resolver](resolver.md) has the rules.

## origin containment

Every module stays inside its [root](#root), checked on its real path:
a relative or absolute import never leaves the importer's root, enters `node_modules` below it,
or enters another root nested in it, and `"exports"` targets and self-references stay inside their package.
[resolver](resolver.md#departures-from-node) has the rules.

## pin

In `spawn`, what a granted program's name resolves to, held for the run:
the file found (on `PATH` for a bare name, beneath the [project root](#project-root) for one holding `/`), its identity, and its real path,
and on `fgro` for a native binary a descriptor on it, which it runs by; a script keeps none and runs by the path it was found at;
on `fgro`, a script at a `node_modules/.bin` name runs instead under a `node` pinned with it, by its real path ([host modules](host_modules.md#spawn-on-fgro)).
On `fgro` a pin also holds the file's change time (`ctime`), checked with its identity as it launches,
so a later change on disk never changes which file a handle runs:
a file replaced, removed, or made at its path since is never what it runs,
and one rewritten in place, which keeps its identity, is refused, as is one whose mode, owner, or links changed,
short of a change within the filesystem clock's tick of the file's last change before the pin.
On Node a script's pin holds its identity alone, so one rewritten in place runs as rewritten,
and a native binary replaced at its path runs the new file ([host modules](host_modules.md#the-node-implementations-gaps)).
`fgro run` pins each program `--allow-run=` names as the run starts, before any module evaluates, a name that finds nothing included,
and `fgro <command>` those and each program its [command's closure](#commands-closure) grants;
under `-A`, and on Node, a name is pinned when a handle is made of it ([host modules](host_modules.md#spawn-on-fgro)).
It isn't a release binary's hash pin, the hash a caller checks an `fgro` file against ([eval](eval.md#the-envelope)).

## policy file

The committed record of the grants accepted in review, designed and provisionally named `gro.policy.json`.
It holds each entry's grants and [run kind](#run-kinds) (for Gro, each task and gen file),
and the project's check (`gro check`) diffs it like a lockfile, failing on a widening until a person accepts it.
It names a spawn grant's programs, while each one's pin, a real path that differs per machine, is designed to live in a local file that isn't committed.

## pre-pass

The binary's native step for `fgro <command>`, taken before any thread starts and before any JS runs.
It reads the [command's closure](#commands-closure), the grant flags after the command word, and the project at the working directory,
decides any [fallback](#fallback) the command line or the project's files decide, and holds no descriptor when it returns
([box](box.md#how-it-goes-up)).

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
  seen per root or package, and the only one of the three the runtime enforces (built for the builtin modules' link-time check, designed for the policy file)
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

Something that reports without enforcing, like the install check `gro check` runs, a grant diff, or the policy file.
The docs and the CLI name each one as a signal, with what evades it, never as a [boundary](#boundary) or a [guardrail](#guardrail).

## root

A directory whose modules share an [origin](#origin): the entry's [project root](#project-root),
or the real directory of a package a bare import [admitted](#admission).
A module's root is the innermost admitted one above its real path,
and a relative or absolute import resolves only inside its importer's root,
never into `node_modules` below it or into another root nested in it.

The `fs` host module's roots on `fgro` are another table: the directories a run reads or writes ([read and write roots](#write-root)),
each opened once and held by a descriptor, which every [handle](#handle) resolves beneath;
the project root is the first, and a temporary directory `fs.temp_dir` makes joins as the run goes ([host modules](host_modules.md#fs-on-fgro)).

## run kinds

Every run is one of three by what it reaches and depends on,
[hermetic](#hermetic), [confined](#confined), or [unconfined](#unconfined),
derived from its grants and recorded in the policy file.

## seal

Making the global object non-extensible after bootstrap, so a handed capability can't be stashed there as a new global;
a package that adds a global needs a policy entry naming it.
Designed ([authority](authority.md#by-construction-or-by-default)); today the global object is extensible.

## security gate

A suite of escape probes run under every grant set,
the way the bench measures speed, to be passed by every milestone after `gro eval`.
Designed; today's probes are `fgro eval`'s acceptance tests.

## self-reference

A bare import of the importer's own package by its name, resolved through that package's `"exports"`
and kept inside it, as Node resolves one ([resolver](resolver.md#specifiers)),
but for Gro's own package, whose specifiers resolve in [Gro's library](#gros-library) whoever imports them.

## tool directory

A directory in the project a tool writes on its own, like `.svelte-kit` or Vitest's `node_modules/.vite/vitest`,
which the run's [box](#box) makes a [write root](#write-root) of each tool whose grant in the grants table names it;
what a tool leaves there is code a later command may run ([box](box.md#what-a-commands-box-admits)).

## type marker

A `package.json` whose keys are all `"type"` or `"sideEffects"` (an empty one too),
like `dist/esm/package.json`. It sets a directory's module format without bounding the project.

## unconfined

The [run kind](#run-kinds) under `-A` or a named unconfined phase (designed),
like Gro's `publish`, `deploy`, `commit`, and `changeset`, which push and use credentials:
every grant at its widest, with no global added and native code still out of process.

## write root

A directory a run may write.
As designed, a project's are declared and accepted like any [grant](#grant) (`dist/`, `build/`, `.gro/`).
In a run of `fgro <command>` the run's [box](#box) sets them ([box](box.md#what-a-commands-box-admits));
on `fgro run` today, a run's write roots are each directory `--allow-write=` names, or `/` under `-A`, and each directory `fs.temp_dir` makes under `--allow-temp`, read and written, until it's disposed of.
A read root is the same for reads. On `fgro`, each read or write root is one of the `fs` module's [roots](#root).
