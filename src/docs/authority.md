# authority

> The runtime's security model: authority by import with nothing ambient, grants and the policy file,
> the three run kinds, what holds by construction and what by default, the threat model, imports or globals, and its open questions

> **status**: the model as designed. Built today: `fgro eval`'s box ([eval](eval.md#the-box))
> and the omissions (no runtime namespace, no Node compat, no remote imports, no authority globals).
> The host modules, the grants, the policy file, the seal, and the kernel backstop for `fgro run` aren't built, so nothing below is enforced for `fgro run` yet;
> the [roadmap](runtime.md#roadmap) says when, and [backstop](backstop.md) has the kernel layer under all of it.

These terms carry this doc, and the [glossary](glossary.md) defines them and the others:

- **host modules** are the imports that reach the OS (fs, spawn, watch, net, and the process modules),
  each a visible edge in the graph ([host modules](host_modules.md)),
  the running process one module per kind of authority it carries, each its own grant
  ([the process modules](host_modules.md#the-process-modules))
- **authority globals** are the web globals that would carry authority
  (the network, storage that outlives the run, a channel found by name, code outside the graph);
  Gro has none, since a global reaches every module in the graph:
  each is an import instead, if Gro implements it (the network is the `net` import),
  and every other web global is pure
- a **grant** is one accepted reach, like a host module, named hosts, or a read root,
  accepted for an entry and held by an [origin](glossary.md#origin), the project or one package,
  so any module of that origin may import what it's granted, and nothing is scoped per module
- the **policy file** (currently `gro.policy.json`) is the committed record of the grants accepted in review
- every run is one of three **kinds** by what it reaches and depends on,
  hermetic (replayable), confined (bounded), or unconfined
- the **kernel backstop** is a coarsening of the grant set enforced as Linux kernel rules (Landlock, seccomp),
  under the runtime's own checks

## Secure by default

Code starts with nothing and gets only what it's granted.
Confinement is the default, with no flag to turn it on; `-A` (`--allow-all`, the [escape hatch](glossary.md#escape-hatch)) turns it off.
Widening is a visible act, reviewed like a lockfile change.
A run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared write roots,
  and the public env names it declares
- **only after it's accepted in review:** the network, by named hosts
  (a blanket grant exists, accepted the same way and shown as the widest class),
  spawning processes, reads outside the project, private env names,
  and any host module a dependency imports
- **never short of `-A`:** `.env*` files except through the host's env reader,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`
- **under `gro eval`:** its own module graph, read-only, with no network, writes, env, or spawn,
  and the JIT off unless a flag turns it on
  (with the JIT off V8 has no `WebAssembly` at all, so a config that uses WASM needs the flag)
- **for native code:** FFI, a grant that loads the library in a confined process of its own,
  not the runtime's, even under `-A`

### How grants are proposed and accepted

A script's needs are a static property of its import graph,
so least privilege needs no hand-written permission list.
A file that imports only `fuz_util` and the fs module can't spawn or fetch.
That's `gro run`'s unit, one file's graph,
and a tool that runs several entries in one invocation, like Gro's tasks,
holds their union ([Gro's workflow](runtime.md#gros-workflow-on-the-runtime)).
The source proposes grants (the host modules it imports, plus a static `grants` literal
naming hosts and the like) and the committed policy file authorizes them,
so a dependency bump that newly imports the filesystem, spawn, or the network
will fail at link time, and in the project's check, until the diff is accepted,
the same shape as a lockfile change.
Inference proposes and the loader enforces: every edge, literal or computed, is checked against the importer's accepted grants when it links,
so the policy holds even where a static walk can't see an `import()` whose specifier is computed,
and an `import()` in code compiled from a string, which has no importer of its own, is refused ([run](run.md#what-loads)).
A run outside any project, or one that needs a grant just once, takes it by flag instead,
in Deno's spelling (`--allow-net=api.example.com`, `--allow-env=A,B`), for that run only and recorded nowhere,
and `-A` is the same flag at its widest.
A bare `--allow-env` is a usage error, not Deno's grant of every name.
The network included: `fetch` arrives by import, as `net.fetch`,
so a dependency that starts fetching is a new edge in the graph, and the diff names the package.

The policy file is also a guardrail for the code agents write, not for the agent itself, which already holds your shell's authority.
What an agent's code may reach sits in one small file a person can audit,
the runtime will enforce it (nothing enforces it today), and a widening is a diff in review.
Keeping an agent from editing that file itself is still open, and for a project without a review process,
"accepted" means someone ran the accept command, which an agent can run.
Review, harness rules, and a push gate cover the normal case but aren't a structural guarantee,
and a signed acceptance is designed for when it's needed
([open questions](#open-questions)).

### Run kinds

Every run is one of three kinds, derived from its grants and recorded in the policy file,
so an entry that stops being hermetic is a reviewed diff.

| kind       | network and spawn | clock and randomness | reads                         | replays |
| ---------- | ----------------- | -------------------- | ----------------------------- | ------- |
| hermetic   | none              | fixed or denied      | under the root, each recorded | exactly |
| confined   | by grant          | real                 | by grant                      | no      |
| unconfined | all (`-A`)        | real                 | all (`-A`)                    | no      |

`fgro eval` is hermetic (with the gaps [eval](eval.md#the-graph) lists), as Gro's gen files are designed to be.
A grant is always a grant of something specific, so granting individually,
even a blanket network grant, keeps a run confined, with a wider reach shown in the diff.
A net grant names hosts, or is a blanket grant.
A hermetic run also gets no storage and no by-name channels, since persistence and cross-run channels break replay.
Only `-A` (`--allow-all`, as in Deno) and the named unconfined phases below run unconfined.
Every spawn grant is shown in the diff as bounded by the kernel backstop alone,
since for most programs the arguments are code (`git -c core.sshCommand=…`, `node -e`) and a handle on the binary bounds the binary, not what it does;
a shell, or a child given the whole environment, is shown as `-A`.
`-A` widens every grant and adds no global: a free `fetch` still fails.

### By construction or by default

The guarantees come in two kinds, and it's worth knowing which is which.
Some hold by construction, with no setting that changes them.
Authority enters by an import, so what a run can reach is a static fact,
though which of its modules ends up holding a capability is not.
There's no `process` global, no Node compat, and no remote import,
and no runtime namespace: the core's own op table is removed from the global object at startup (built),
and each host op checks its grant again as its first step (built, held by tests rather than the compiler), so no leaked reference to the core reaches around the loader.
A WASM module can reach only what its instantiator hands it, and one imported as an ES module declares that statically.
The rest hold by default, where a policy entry or `-A` widens them and the diff shows it.
These are the read and write roots, the net hosts, the deny overlays,
the cleared environment with a small base set for children, the kernel rules, whether a run stays hermetic,
and native code kept out of the runtime's process.
A widening of the second kind is a diff accepted in review, and the first kind has nothing to widen.

The exceptions are explicit and labeled.
`-A` is the one flag that widens everything, never a default; a few grants are as strong (a shell, a child's whole environment) and are shown that way.
A tool built on the runtime can name unconfined phases, like Gro's `publish` and `deploy`,
which push and use credentials;
they're shown that way in the policy diff, with credentials passed to the one process that needs them.

The global object is the one dynamic channel, and it's sealed (made non-extensible) after bootstrap (designed),
so a handed capability can't be stashed there as a new global.
A package that adds a global (a polyfill, say) needs a policy entry naming it,
and the names that carry authority are refused, so routing a capability back into a global fails loudly.
Like any global, an added one's effect covers the whole invocation.
Existing globals and the shared prototypes stay mutable, as in browsers
([open questions](#open-questions) has the shared-prototype route back to a free `fetch`).
A reference parked on one is handing, which can widen what other modules reach but never what its holder could,
and a module that patches a shared prototype can interpose on calls that pass through it,
and so obtain capabilities it was never handed.
Against a dependency treated as hostile the realm isn't the boundary,
and freezing the intrinsics is a hardening the design hasn't made the default ([open questions](#open-questions)).

### Friction and costs

Security-first runtimes earned a reputation for friction,
with flags on every run, prompts mid-execution, and `-A` as the habit that follows.
Gro is designed to have none of those (none of it is built yet, and today a run has no authority to widen).
A normal run in a project has no flags, and code that only reads its project's files and writes its declared write roots has nothing to declare,
and the one moment worth a human's attention, when the graph newly reaches the network,
a process, or files outside the project, arrives as a diff with a copyable fix, never a prompt;
a link-time denial fires before any confined code has written to your terminal, and the fix is a fixed command that recomputes the diff itself
([backstop](backstop.md#the-terminal) has why that matters).
The costs are elsewhere, and stated plainly:
no Node compat and no runtime namespace, so code written for Node or Deno needs porting;
strict ESM, so an npm package without `exports`, or one that's CommonJS, is refused;
TypeScript's erasable syntax only;
and no global carries authority, so a package that reaches the network only through the global `fetch` isn't supported.

## Threat model

Built today: `fgro eval`'s box; `fgro run` has no backstop yet, so an engine exploit in a run isn't contained.

Grants are designed to contain overreach.
A dependency, a task, or an agent that reaches for authority nobody accepted
fails before it runs (an unaccepted host module fails at link time,
and there's no global to reach around it), and the widening shows up as a diff in review.
V8 itself isn't treated as the boundary against code built to escape it.
An engine exploit lands in the kernel backstop,
which applies a coarsening of the same grants to the whole invocation and every child process,
and `gro eval` and FFI each run in a process of their own.
Within one realm, a package's own grants catch overreach and attribute it in review,
but a dependency built to steal can patch a shared prototype
and catch a capability on its way through project code,
so against it the bound is the invocation's whole grant set and the kernel backstop:
attribution catches the careless, and the invocation's grants plus the kernel box bound the deliberate.
Code that must be treated as hostile belongs in WASM or a separate process,
not under a looser grant.
WASM runs with V8's JIT on, so against an engine exploit
what holds it is the kernel backstop, not the engine.
Until the Node tools leave the loop, which is the distant end of the roadmap, an invocation that spawns them
is confined only as finely as the kernel backstop, by paths and ports rather than the grant set;
that is the common verb, `gro check`, until then, and the strong claims above describe `gro run` and the dependency's own graph.
A child process is where a box is weakest, since a program's arguments and its config files are code,
so children get the project read-only and only the declared write roots writable, at the kernel
([backstop](backstop.md#the-box-for-runs-and-children) has the rules and what they close).
Availability is outside the model: a confined run can spend CPU, memory, and descriptors, and fill its writable roots;
`fgro eval` caps its own, and `fgro run` has no limits yet ([backstop](backstop.md#resource-limits)).
The terminal is a channel: confined output can carry escape sequences, so the host is designed to escape a run's own console output
(a child's inherited terminal stays unescaped, and today `fgro` escapes none, [run](run.md#usage)) and never to print a grant diff inside a run's output
([backstop](backstop.md#the-terminal)).
Outside the model are `-A` and the named unconfined phases (Gro's `publish` and `deploy`), which run the build they contain unconfined too.
Linux is the design target, and the kernel backstop and every claim that rests on it are Linux's.
On macOS and Windows anything past the runtime's own checks is outside the model
until each gets a sandbox design of its own, never a translation of Linux's.

## Imports or globals

Authority could reach code two ways, and Gro has only the first. The table is why.

|                    | a host module import (`net.fetch`, fs, spawn)            | an authority global (`fetch`, `WebSocket`), as Node, Deno, and Bun have |
| ------------------ | -------------------------------------------------------- | ------------------------------------------------- |
| granted            | inferred from the import, accepted in review             | declared by hand, since any expression can name a global |
| reaches            | the modules it's imported into and handed to             | every module in the invocation                    |
| the diff shows     | which package newly reaches it                           | that the invocation needs it, not which code uses it |
| narrowed           | on the value (fewer hosts, a subdirectory), passed down  | to the accepted hosts, the same for every module  |
| when denied        | the import fails at link time                            | it's absent; a scan for free references would be evadable |
| portable           | the Node twin in Gro's npm package until binary-only     | anywhere the web API exists                       |
| existing packages  | the ones written to take it                              | the ones that call the global today               |
| in the code        | passed down the call graph                               | reachable from anywhere                           |

With no authority global, every reach starts at an import edge, so inference proposes the whole grant set,
the loader enforces it at every link, and the policy diff attributes every newly imported authority to a package.
Both columns describe code that doesn't patch shared prototypes;
against a dependency that does, the [threat model](#threat-model) says where the bound is.

The host's net module exports a `net` handle (`net.fetch`, with `net.WebSocket` and `net.EventSource` when a consumer brings them,
bound to the accepted hosts) that's narrowed and passed down like any other capability,
so authority is visible at every use site.
There's no global `fetch` to fall back to: a free `fetch` is a `ReferenceError` (today a plain one; the hint naming the import is designed).
The net grant is inferred from the graph like a filesystem one, and the hosts it reaches are declared.
Pure globals (`URL`, `TextEncoder`, streams, `console`) are always there,
and the scoped checks inside a grant (a path outside a root, a host not accepted) are the host's, at the call,
and designed to be reported so a `try/catch` can't hide them (today a caught one is silent).
An `import()` refused by rule, an ungranted host module's among them, is reported after an `fgro run` even when caught
([run](run.md#errors-and-exit-codes)); the host ops' scoped refusals join it once ops can refuse.

Where the import happens matters: at the [composition root](glossary.md#composition-root),
the task or entry module where authority enters, with library code taking capabilities as parameters
and the narrowest handle (a directory handle, `net.narrow({hosts})`) handed down.
[host modules](host_modules.md#where-to-import-them) has the how-to.
`net.fetch` is bound, so it can be handed to any library that accepts an injected `fetch`,
and `const {fetch} = net` gives the bare name in one module; `net.WebSocket` and `net.EventSource`, once a consumer brings them, hand the same way.

A single-file script is its own root and imports directly.
The cost is that the policy sees [reach](glossary.md#reach-need-and-use) only at the roots.
What a library needs shows in its parameter types, what it used shows in the [access manifest](glossary.md#access-manifest),
and what the root hands to whom is in the root's code, not the policy file.
Nothing mechanical checks that hand, so a root passing a whole handle where a narrowed one would do raises no diff.
What it buys is a bound per callee rather than per origin, like one subdirectory or one of the accepted hosts,
and it binds only code whose origin isn't granted the host module itself:
a dependency holds what it's handed, while a project module can import whatever the project is granted,
so for first-party code a narrowed handle is a convention review reads, not a limit.
A grant limits what code can obtain, never what it's given,
so even enforced per package it bounds the importer, not the code the importer hands to.

The cost is the authority-global column's "existing packages" row.
A package that reaches the network only through the global `fetch`, and can't take an injected one,
doesn't run on Gro, and there's no compat layer for it:
the fix is a package that accepts `fetch` as a parameter, or a fork that does.
Code that must also run in browsers takes `fetch` as a parameter the same way.
Security sits above web-standards conformance in Gro's priorities, and this is where the order shows.

Clock and randomness stay globals for a different reason.
`Date.now()` and `Math.random()` are everywhere and can't be removed,
and determinism is a property of the whole run,
so they're set per run kind rather than granted.

## Open questions

Security is the north star, and it isn't finished.
The design doesn't have every answer, and these are the ones it knows it's missing
(the kernel layer's own are in [backstop](backstop.md#open)):

- **Holding a package to its own scope.** A scoped grant accepted for one package (its hosts, its roots)
  bounds that package only if its import yields a handle bound to that scope,
  an instance of the host module per origin, where one shared instance would hold the run's union.
  That instance is the design's direction and isn't built; how a host call is attributed more finely than its entry is open with it.
- **Keeping an agent out of the policy file.** Review, harness rules, and a push gate cover the
  normal case, and a signed acceptance that needs a person's presence is designed, built only if
  the measured widening rate makes its friction worth it.
- **Replay needs deterministic scheduling.** `fgro eval` only loads modules, which ESM orders,
  but a hermetic run that awaits reads (a gen file, say) has them complete on the event loop in no fixed order,
  so it can resolve promises differently between runs.
  The host has to order completions for "replays exactly" to hold by construction,
  and the cost is unmeasured.
- **Interposition within one realm.** Grants are attributed per package, since all authority enters by import,
  but one realm has one set of shared prototypes,
  so a same-realm dependency can patch an intrinsic
  to intercept a holder's calls and the capabilities passing through them.
  A V8 context per package or trust tier, with frozen intrinsics, would close that
  (the shape [LavaMoat](https://github.com/LavaMoat/LavaMoat) builds on SES compartments),
  at the cost of realm identity (`instanceof` across contexts), memory,
  and packages that break under it.
  Separate realms are worth exploring as an opt-in tier, but won't be the default,
  since their startup, memory, and compatibility costs weigh against speed and ergonomics.
  What the opt-in tier looks like is undecided, the limit is stated in the meantime, and the gate suite carries a probe either way.
- **Freezing the intrinsics in the one realm.** Separate realms are the costly option above; freezing the shared prototypes
  in the single realm needs no second realm and the seal already has a named-exception mechanism,
  so whether lockdown is the default for third-party and agent-authored code, with a measured compatibility cost, is a separate decision, undecided.
- **Free names through the global's prototype.** `Object.prototype.fetch = …` makes a free `fetch` resolve
  without touching the seal; how to close that, and at what cost to compatibility, is undecided.
- **Which files under the project are secrets.** `.env*`, credential files, and `.git/config` are denied by the read overlay;
  a project `.npmrc` token, `*.pem`, `.git/objects` (every secret ever committed), and the depth the overlay applies at are not yet listed,
  and a pre-accepted write to a build root plus an unconfined `deploy` is a path out for any of them.
- **Where the builtins' grants live**, for Gro's own workflow ([runtime](runtime.md#gros-workflow-on-the-runtime)).
