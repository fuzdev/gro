# authority

> The runtime's security model: authority by import with nothing ambient, grants and the policy file,
> the three run kinds, what holds by construction and what by default, the threat model, imports or globals, and its open questions

> **status**: the model as designed, part of it built. Built today: `fgro eval`'s box ([eval](eval.md#the-box)),
> the omissions (no runtime namespace, no Node compat, no remote imports, no authority globals),
> and for `fgro run` the link-time grant check by origin over the builtin modules built so far:
> the project's modules get `args`, `exit`, `stdout` and `stderr` (raw writes, beside the console, which no grant gates), `env` with the variables `--allow-env=` names, and `fs`, reading the project's tree
> and the directories `--allow-read=` names, with a write scope on those `--allow-write=` names,
> each scope carried by the handles made under it;
> the grant-free modules link for every module; `-A` grants every module every one;
> and `spawn` with the programs `--allow-run=` names, each pinned as the run starts, and those found beneath the exec roots `--allow-run-in=` names, each held as the run starts,
> its `run`, `out`, `out_bytes`, and `start` starting them unboxed,
> any program granted that way being close to `-A` until each child is boxed, since most can be made to run code by their arguments or environment
> (a shell, git, and cargo among them) and the root's `env` values set any variable: hand library code a narrowed root, which refuses them;
> an exec root that is also a write root runs whatever the run writes there, so a run granted both can write a program and run it, and holds what that program can reach;
> and a refused `import()`, `env` name, `fs` reach, or `spawn` program fails the run, caught or not ([run](run.md#errors-and-exit-codes)).
> `fgro <command>` runs Gro's own commands under the same check, in the run's kernel box.
> Its grants are the command's closure, read natively and joined to the grant flags, and each program the closure grants gets the grants table's environment policy.
> The tools that pass the whole environment (svelte-kit, vitest, eslint, svelte-check, tsc, and npm) run the project's code inside that box,
> bounded by its rules rather than by the grants ([box](box.md#what-a-commands-box-admits)).
> Not built: the box for `fgro run`, where a granted program runs unboxed, `net`, `watch`, `stdin`, the policy file, the deny overlays, and the seal;
> the [roadmap](runtime.md#roadmap) says when, and [box](box.md) has the kernel layer under all of it.

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
- the **policy file** (designed, provisionally named `gro.policy.json`) is the committed record of the grants accepted in review
- every run is one of three **kinds** by what it reaches and depends on,
  hermetic (replayable), confined (bounded), or unconfined
- a **box** is the kernel rules one process is held in, with every program it starts:
  a coarsening of the grant set enforced as Linux kernel rules (Landlock, seccomp), under the runtime's own checks
  ([box](box.md) has that kernel layer),
  built as `fgro eval`'s box and as the run's box, which every command of Gro's the binary runs starts in

## Secure by default

Code starts with nothing and gets only what it's granted.
Confinement is the default, with no flag to turn it on; `-A` (the [escape hatch](glossary.md#escape-hatch), Deno's `--allow-all`, which `fgro` spells only `-A`) turns it off for `fgro run`.
A command of Gro's the binary runs itself refuses `-A`, so its run is always boxed, and the Node Gro (`node_modules/.bin/gro`) is the one unconfined way to run it
(a command the binary leaves to Node passes `-A` to the Node Gro).
Widening is a visible act, reviewed like a lockfile change.
As designed, a run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared write roots,
  and the public env names it declares
- **only after it's accepted in review:** the network, by named hosts
  (a blanket grant exists, accepted the same way and shown as the widest class),
  spawning processes, reads outside the project, private env names,
  and any host module a dependency imports
- **never short of `-A`:** `.env*` files except through the host's env reader,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`,
  the [deny overlays](glossary.md#deny-overlays), designed for after the first release
  (its box leaves the project's `.env*` files and `.git/config` readable
  and the directories tools own in `node_modules` writable, [threat model](#what-the-first-release-is-for))
- **never, `-A` included:** reads of a process's environment, memory, and command line
  (`/proc/*/environ`, `/proc/*/mem`, `/proc/*/cmdline`), the always-on part of the read-deny overlay,
  a check of the runtime's own `fs` (designed); the kernel box a command of Gro's runs in lets its tools read every process's command line,
  and refuses only the environment, memory, and working directory of a process outside it ([box](box.md#what-a-commands-box-admits))
- **under `gro eval`:** the module files of its project and linked packages, read-only (the kernel admits those trees whole today, [eval](eval.md#the-box)), with no network, writes, env, or spawn,
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

Against the code agents write, a widening of the policy file is a [review signal](glossary.md#review-signal), a diff someone has to read,
and against the agent itself it's nothing, since the agent already holds your shell's authority.
What an agent's code may reach sits in one small file a person can audit,
the runtime will enforce what it records (nothing enforces it today), and a widening is a diff in review.
Keeping an agent from editing that file itself is still open, and for a project without a review process,
"accepted" means someone ran the accept command, which an agent can run.
Review, harness rules, and a push gate cover the normal case but aren't a structural guarantee,
and a signed acceptance is designed for when it's needed
([open questions](#open-questions)).

### Run kinds

As designed, every run is one of three kinds, derived from its grants and recorded in the policy file,
so an entry that stops being hermetic is a reviewed diff;
today `fgro eval` is the hermetic one and `fgro run` is confined, or unconfined under `-A`.

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
Only `-A` and the named unconfined phases below run unconfined.
Every spawn grant is shown in the diff as bounded by its [box](glossary.md#box) alone,
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
and each host op and native member checks its grant again as its first step (built, held by tests rather than the compiler), so no leaked reference to the core reaches around the loader.
A WASM module can reach only what its instantiator hands it, and one imported as an ES module declares that statically.
The rest hold by default, where a policy entry or `-A` widens them and the diff shows it.
These are the read and write roots, the net hosts, the deny overlays (designed; all but the always-on one on `/proc/*/environ`, `mem`, and `cmdline`, which nothing widens),
the cleared environment with a small base set for children, the kernel rules, and whether a run stays hermetic.
Native code stays out of the runtime's process, `-A` included: FFI loads its library in a confined process of its own.
A widening of the second kind is a diff accepted in review, and the first kind has nothing to widen.

The exceptions are explicit and labeled.
`-A` is the one flag that widens everything but the always-on deny overlay (designed), never a default; some grants are as strong (any program until children are boxed, a child's whole environment) and are shown that way.
As designed, a tool built on the runtime names unconfined phases, like Gro's `publish`, `deploy`, `commit`, and `changeset`,
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
A builtin module's own code passes through none of them: it runs when a run first imports the module, which can be after such a patch,
and it takes every intrinsic it uses from references the runtime took before any user code ran, using none whose work reads a prototype such a patch can reach ([host modules](host_modules.md#native-requirements)).
Against a dependency treated as hostile the realm isn't the boundary,
and freezing the intrinsics, adopted for confined runs, isn't built ([open questions](#open-questions)).

### Friction and costs

Security-first runtimes earned a reputation for friction,
with flags on every run, prompts mid-execution, and `-A` as the habit that follows.
Gro is designed to have none of those (today `fgro run` has no prompts and widens only by its one-run flags, which [run](run.md#usage) lists; the policy file isn't built).
A normal run in a project has no flags, and code that only reads its project's files and writes its declared write roots has nothing to declare,
and the one moment worth a human's attention, when the graph newly reaches the network,
a process, or files outside the project, arrives as a diff with a copyable fix, never a prompt;
a link-time denial fires before any confined code has written to your terminal, and the fix is a fixed command that recomputes the diff itself
([box](box.md#the-terminal) has why that matters).
The costs are elsewhere, and stated plainly:
no Node compat and no runtime namespace, so code written for Node or Deno needs porting;
strict ESM, so an npm package without `exports`, or one that's CommonJS, is refused;
TypeScript's erasable syntax only;
and no global carries authority, so a package that reaches the network only through the global `fetch` isn't supported.

## Threat model

Built today: `fgro eval`'s box, the run's box for `fgro <command>`, and for `fgro run` and `fgro <command>` the link-time grant check over the builtin modules built so far,
each handle's scope, and the pinned programs.
`fgro run` has no box yet, so an engine exploit there isn't contained,
and a command run on the Node Gro (`gro check` there, or a [fallback](glossary.md#fallback)) confines nothing.
The rest of this section is the model as designed, with what's built marked.

### Three kinds of protection

Each mechanism in these docs is named as one of three kinds, with what defeats it:

- a [boundary](glossary.md#boundary) holds against code built to defeat it, because it's a native check, a separate process, or the kernel,
  like the link-time grant check, a handle's scope, a pin, or a [box](glossary.md#box)
- a [guardrail](glossary.md#guardrail) is enforced in the shared realm or by convention, and holds against mistakes, not intent,
  like a builtin's `host`, a narrowed handle in first-party code, or the frozen grants table
- a [review signal](glossary.md#review-signal) reports without enforcing,
  like the install check `gro check` runs, or a grant diff and the policy file (both designed)

### What it protects, and whom it trusts

The assets are secrets in the environment and in files (`.env*`, credential files under the home directory, `.git/config`),
the user's files outside the project, the network, and the terminal with whatever logs it, CI's public logs included.
The project's own tree is one too, since later commands run it, some of them unconfined,
like `.git`'s hooks and config, `node_modules`, config files, `package.json` scripts, and the sources themselves.

Gro trusts its own code and the kernel,
the pinned programs as binaries but never for what their arguments and config files make them do,
V8 for correctness but not as a boundary,
and whoever accepts the policy file.

### Who it's against

- Gro's own bug is caught first by guardrails, like a builtin's `host` and the test that holds what Gro's own modules may import,
  and then by whatever bounds the adversaries below.
- Careless code, a dependency, task, genfile, or agent-written module that reaches further than it should without evading,
  is refused at link time or at the call (built for `fgro run` and `fgro <command>`), its widening attributed in the policy diff (designed),
  and guardrails catch it too.
- A dependency built to steal holds only what it's handed, since its origin is granted nothing unless accepted
  (built for `fgro run` and `fgro <command>`, where a package is granted nothing short of `-A`).
  While the shared prototypes stay mutable it can patch one and catch a capability passing through project code,
  and then its bound is the invocation's whole grant set.
- Deliberate code in the project holds the invocation's whole grant set.
  The native checks and the box (built for `fgro <command>`, designed for `fgro run`) bound it, and guardrails don't.
- A child process, whose arguments and config files are code, is bounded by its box:
  under `fgro <command>` a narrower box of its own inside the run's box (both built).
  Under `fgro run`, which has no box yet, a program a run is granted is close to `-A`.
- An engine exploit is bounded by the box of the process it lands in, built for `fgro eval` and `fgro <command>` and designed for `fgro run`,
  so today an exploit in `fgro run` reaches what your user can.

Under `gro <cmd>` on the binary (built, in the run's box), the invocation's grant set is the command's,
the CLI's own grants, every builtin in the command's closure with its programs and variables, and the grant flags.
Each builtin's `host` is cut from that set by the builtin's own grants, a guardrail against Gro's own mistakes.
The project's code shares the realm. On the binary no project task runs (`gro task <path>` runs on the Node Gro),
but the config and genfiles run as the project's own modules, at its [origin](glossary.md#origin) by their paths, imported through the [generated entry](glossary.md#generated-entry), and import any host module the set holds,
and a package they import, granted nothing itself, can patch a shared prototype a builtin's handles pass through,
so that code holds the command's whole set ([task](task.md#a-builtins-host-and-grants)).
The Node tools a command spawns, until they leave the loop at the distant end of the [roadmap](runtime.md#roadmap),
and git, which runs a command a repository's own config names (`core.fsmonitor`, on `status`, `ls-files`, and `diff`),
are held by the box, by paths and the network rather than the grant set,
so the native checks' finer claims describe `fgro run` and a dependency's own graph,
and `gro check` is confined only as finely as the box ([box](box.md#what-a-commands-box-admits)).

Code that must be treated as hostile belongs in WASM or a separate process, not under a looser grant.
WASM runs with V8's JIT on, so against an engine exploit what's designed to hold it is the box, not the engine,
and FFI is designed to load its library in a process of its own, as `gro eval` runs in one.
The terminal is a channel too: confined output can carry escape sequences, so the host is designed to escape a run's own console output
(a child's inherited terminal stays unescaped, and today `fgro` escapes none, [run](run.md#usage)) and never to print a grant diff inside a run's output
([box](box.md#the-terminal)).

### The time-shifted write

What a command writes into the project is code the next unconfined command may run,
like `gro dev`, an npm script, git's hooks, or the editor,
so a write grant on the project is as wide as that command's reach.
A plain `gro check`, without `--install` or `--build`, writes no source on either runtime.
It runs `gen` and `format` with `--check`, and verifies the install and `package.json` without writing them, failing with the fix
([task](task.md#a-builtins-host-and-grants)).
`gro gen --check` and `gro format --check` write none either.
On Node that's what Gro's own code does, not a protection, since the config, a genfile, or a test it runs can write anything your user can.
On the binary the `check` marks are data the box takes its write roots from.
The [command's closure](glossary.md#commands-closure), read natively before any JS runs,
drops the writes on the project of `check`'s `gen` and `format`, and of a `gen --check` or `format --check` run on its own,
so in the project a boxed `check` writes only its [tool directories](glossary.md#tool-directory), like `.svelte-kit`, fuz_css's `.fuz/cache`, and the ones tools write in `node_modules`,
a boxed `gen --check` no more than that, and a boxed `format --check` nothing,
and a test or genfile any of them runs can't rewrite a source, a workflow, or `package.json`.
A command granted a write on the project, like `gen`, `format`, or `sync`, writes each top-level entry but `.git`, `node_modules`, and the other dot-entries,
and reaches as far as the next unconfined command that runs what it wrote.

### What the first release is for

The first release of Gro on its runtime is for local and agent runs of the check commands
(`check`, `test`, `typecheck`, `lint`, `format`, `gen`, and `sync`) on a Linux workstation,
for people and agents running code they haven't vetted.
Every other command, like `dev`, `build`, and `clean`, keeps running on the Node Gro, and so does `check --build`, which CI's workflows run,
and install runs outside any run (built), so an asset one of those gives away isn't one the release protects.
When the project's installed Gro is a different version from the binary's, the binary runs the project's Node Gro and says why (built).
The project's own files decide that fallback, so a project can leave the box by pinning another version.
Where the kernel's Landlock is missing, off, or below its sixth ABI (Linux 6.12), or a seccomp filter refuses its calls, there's no box to start in,
so the binary runs the Node Gro there too, and says why (built):
the host decides that fallback, not the command line or the project.
`GRO_FALLBACK=command` refuses both, and every other fallback the project's files or the host decide, before anything runs (built), for an agent's sandbox to set:
read from the process's environment alone, never the project's files, it's a [boundary](glossary.md#boundary) against a project pinning its way out
and against a sandbox running unconfined because of the kernel it landed on,
defeated only by unsetting it or by running the project's Gro another way (`node_modules/.bin/gro`, a package script), each the user's choice.
`GRO_FALLBACK=all`, or unset, takes every fallback, and any other value is refused.
Under `GRO_FALLBACK=command` a fallback the command line decides, like `dev` or a project task, still runs the project's installed Gro from its `dist`,
which the project controls and which confines nothing.
The `note:` line a fallback prints is a [review signal](glossary.md#review-signal), which the project's author controls.

What the release claims, on Linux with Landlock's sixth ABI:
every command of Gro's that the binary runs runs in a kernel [box](glossary.md#box) (built),
covering Gro's code, the project's config and genfiles, and the tools it spawns, which inherit it,
and each tool it spawns runs in a narrower box of its own inside it, writing only its own directories
(both boxes built and installed; each tool of the grants table writes the command's tool directories, and each tool what `--allow-run-write=` names for it,
and its writing only its own directories is designed, which the release waits for, [box](box.md#a-box-for-each-tool)).
Nothing in the home directory is readable but the project's own trees wherever they sit, what git reads there, and the programs the run pins, like `node`'s own file where a version manager installed it there:
git's global config, its ignore and attributes files, and the files its config includes or names as `core.excludesFile` and `core.attributesFile`,
never git's credential store unless your own git config names it as one of those.
Beside those the box reads only the system's files, `/proc`, the project, the packages linked into it, a `node_modules` it links from another checkout,
git's own directories for a linked worktree, a submodule, or a repository the project sits inside below the home directory,
the run's private temp directory, and what a grant flag names.
The network is a private one with only loopback, so a test that binds `127.0.0.1` still passes.
Where the system restricts unprivileged user namespaces, the box refuses internet sockets instead, loopback's included, so such a test fails there, and a `note:` line says so.
The project is read-only but for the command's write roots: under `gen`, `format`, and `sync` each top-level entry but `.git`, `node_modules`, and the other dot-entries,
and under every command that starts a tool from the project the [tool directories](glossary.md#tool-directory) its dependencies call for, like `.svelte-kit` and fuz_css's `.fuz/cache`.
The project's own `.git` and `node_modules` are never writable but for the tool directories in `node_modules` (`$app`, `.vite`, and `.vite-temp`), unless a grant flag names them.
A tool and its children may execute the system's programs, the programs the run pins, `node` among them, the project's `node_modules` and the packages linked into it,
a `node_modules` linked from another checkout, the run's private temp directory, and what `--allow-run-in=` names.
Inside the box the per-builtin grants are guardrails.
[box](box.md#what-a-commands-box-admits) has the box's exact rules.

The box is a boundary, the kernel's, against the project's code and tests, the tools and their children, and an engine exploit;
the `note:` lines and refusals it prints are review signals, which the run's code can imitate.
What the release leaves is stated with it:

- a process in the box can still change the mode, times, and attribute flags of a file your user owns,
  like making a secret readable to a group or a disabled hook executable,
  each harm needing a later unboxed step that an unboxed `dev` or install already gives away;
  changes of ownership and extended attributes are refused
- a path's metadata, whether it exists, its size and times, and a link's target, stays readable everywhere, the home directory included,
  and an inotify watch tells the names of entries made, changed, and removed in any directory your user can reach
- `.env*` files in the project stay readable to every process in the box, and so does any other secret kept in the tree,
  since `dev` and `build` read them unconfined anyway; the read-deny overlay that would refuse them is designed, not part of the release
- `/proc` stays readable, so boxed code sees the command line and status of every process your user can see, as an unconfined `dev` does;
  the environment, memory, and working directory of a process outside the box stay refused,
  and each tool runs in a box of its own, so the tools never read one another's environment, memory, or working directory;
  a tool still sees the others' command line, status, and descriptor numbers, gets a handle from `pidfd_open` on another or on the run, one that can't signal,
  reads the others' write roots, and can take the terminal's foreground
- a hard link made outside the run reads as the file it links, under whatever name it has
- the tools see the run's whole environment, as on Node, and what they print can carry it
- git's global config stays readable to every tool, with any credential or token kept in it
- a write root is code the next unconfined command may run, as above:
  a tool directory's contents run later, like the optimized dependencies in `node_modules/.vite` that `dev` loads,
  and under a write command a `.git` or `node_modules` nested in an included directory stays writable, which Landlock can't carve out
- every tool of the grants table writes every tool directory of the command, so its code can plant in another's;
  even once each writes only its own, a test can write Vitest's own directories, which a dev server already running picks up,
  and the project's config and genfiles run in every tool
- a directory `--allow-run-write=` names for a tool is written by everything that tool runs, its config, its plugins, and for a test runner the tests;
  code in the run can start that tool with arguments of its own;
  and within one `check`, a test given a directory under `src/` can write a genfile that the same run's `gen --check` loads in Gro's own code, which holds every tool of the command with its writes
- a child that sets `PR_SET_PDEATHSIG` while the short-lived thread that started it in its box still runs is killed when that thread ends
- `--allow-read=` and `--allow-run-in=` widen every box, `--allow-write=` the run's box and Gro's grants but no tool's, and `--allow-run-write=` one tool's box and the run's alone,
  so `--allow-write=.git` lets Gro's own code write `.git`, and `--allow-run-write=git:.git` lets git,
  and a `node_modules` linked from another checkout is read there, by the box and by Gro's own code alike, and written there, its tool directories included, only when a write flag names where it leads
- `-A` after a command word is refused, so the unconfined way to run a command is the Node Gro (`node_modules/.bin/gro`), which confines nothing
- `fgro run` has no box yet, so a script run with it, and every program it's granted, is bounded by the runtime's own checks alone
- a project that pins another Gro version runs on the Node Gro, unboxed, unless `GRO_FALLBACK=command` is set
- where Landlock is missing, off, or below its sixth ABI there's no box, and the binary runs the Node Gro, which confines nothing, and says why, unless `GRO_FALLBACK=command` is set, which refuses it
- a `node` that's a script, like an asdf or nodenv shim, is refused before the box, and a native shim, like volta's, can't be told apart from `node` and fails inside it

### Outside the model

- a process outside the run that can already rename or replace the project's root or what a grant flag names,
  since a flag's directory is held to the real path, identity, and nesting it had as the run started, the project's root to none
  ([host modules](host_modules.md#fs-on-fgro)), and nothing more is claimed against such a process
- a second run cooperating with the first
- install time, since `npm install` and its scripts run outside any run
- `-A`, the Node Gro run on its own (`node_modules/.bin/gro`, a package script), and the named unconfined phases (Gro's `publish`, `deploy`, `commit`, and `changeset`),
  where `publish` and `deploy` run their build unconfined too
- availability, since a confined run can spend CPU, memory, and descriptors, and fill its writable roots;
  `fgro eval` caps its own, and `fgro run` has no limits yet ([box](box.md#resource-limits))
- macOS and Windows, since Linux is the design target and the box and every claim resting on it are Linux's,
  so on those anything past the runtime's own checks is outside the model
  until each gets a sandbox design of its own, never a translation of Linux's

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
and the scoped checks inside a grant (a variable outside a handle's names, a path outside a root, a host not accepted) are the host's, at the call.
An `import()` refused by rule, an ungranted host module's among them, fails an `fgro run` even when caught, and is listed after it
([run](run.md#errors-and-exit-codes)), and so does a host function's `not_granted`, built for `env`, `fs`, and `spawn`, listed beside it, so a `try/catch` can't hide either;
the other scoped checks are designed to do the same as their modules land.

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
so for first-party code a narrowed handle is a [guardrail](glossary.md#guardrail) against mistakes, not a limit.
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
The working directory, the process's id, and the runtime's version are on that axis too, though they're imports
(`cwd`, `pid`, and `runtime_version`, [host modules](host_modules.md#the-process-modules)): none reaches anything, so none takes a grant,
and a package's module imports them as the project's does (built on `fgro`, [host modules](host_modules.md#the-modules)), while an import of one is a fact of the graph a run kind can act on (designed).

## Open questions

Security is the north star, and it isn't finished.
The design doesn't have every answer, and these are the ones it knows it's missing
(the kernel layer's own are in [box](box.md#open)):

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
  in the single realm needs no second realm, and the seal already has a named-exception mechanism.
  Lockdown, that freezing, is adopted for every confined run, prototyped and built after the first release.
  How lockdown meets the seal, and how a package that patches a prototype opts out at a measured compatibility cost, stay open.
- **Free names through the global's prototype.** `Object.prototype.fetch = …` makes a free `fetch` resolve
  without touching the seal; how to close that, and at what cost to compatibility, is undecided.
- **Which files under the project are secrets.** `.env*`, credential files, and `.git/config` are designed to be denied by the read overlay, which isn't built;
  a project `.npmrc` token, `*.pem`, `.git/objects` (every secret ever committed), and the depth the overlay applies at are not yet listed,
  and a pre-accepted write to a build root plus an unconfined `deploy` is a path out for any of them.
