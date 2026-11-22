# backstop

> The kernel backstop under the runtime's own checks: what Landlock and seccomp each hold and can't,
> the one namespace the box takes, the box designed for runs and their child processes, the terminal as a channel, and resource limits

> **status**: `fgro eval`'s box is built ([eval](eval.md#the-box) has its exact walls).
> The run's box is built in parts, its Landlock rules as data, its seccomp denylist, the steps it takes before it goes up,
> and the type state that installs them in order, and no command installs it yet,
> so `fgro run` and Gro's commands run with no kernel box and no resource limits today ([run](run.md)).
> The rest here is designed, not built.

The runtime checks authority twice.
Its own checks are fine-grained: a file carved out of a tree, a hostname, an environment variable by name.
The kernel's are coarse: directory hierarchies, TCP ports, syscall numbers.
So the kernel layer is a coarsening of the grant set, installed under the runtime's checks,
and it's what an op-check bug, an engine escape, or a spawned tool lands in.
It never replaces the runtime's checks, and it keeps the user's uid: there's no second account behind it,
so a gap in its rules reaches whatever the user can, and reading one credential file is a full escape.

## What Landlock holds

Landlock restricts the thread that installs its rules, and every process that thread starts, and the rules are never widened.
Its filesystem rules grant rights beneath a directory the process has opened,
so what a rule covers is a hierarchy, and the kernel resolves each path the process opens:
a symlink inside a granted hierarchy that points outside it is refused, since the target's place in the tree is what counts.
What Landlock can't do shapes the rest of the design:

- **It only adds.** There's no deny rule, so `.git` or `node_modules` can't be carved out of a granted project root.
  The alternative is to grant the writable subtrees one by one, which is what the run's box does ([below](#the-box-for-runs-and-children)).
- **Metadata stays readable everywhere**: `stat`, a symlink's target, and so whether a path exists.
- **A hard link is the file.** One made outside the box (a dependency's install script, an unconfined phase) inside a granted hierarchy
  reads like any other file under whatever name it has, so a name-based deny (`.env*`, credential files) can't see it;
  a confined process can't make one across hierarchies itself, since linking needs the `Refer` right on both sides.
- **Its network rules are TCP ports**, bind and connect, with no addresses, no hostnames, and no UDP.
- **Pathname unix sockets** aren't gated until the ninth ABI (Linux 7.1); abstract ones are scoped from the sixth.
- **It only tightens.** A thread's domain can gain layers but never lose one, and a child inherits it;
  the box goes up from the one thread before V8 starts any, so an invocation with an unconfined phase is unboxed from start to finish.

The ABIs the design leans on, with the kernel each arrived in:
filesystem rules (the first, Linux 5.13), linking and renaming across hierarchies (the second, 5.19), truncation (the third, 6.2),
TCP bind and connect by port (the fourth, 6.7), device ioctls (the fifth, 6.10),
signal and abstract-unix-socket scopes (the sixth, 6.12, the floor `fgro eval` and the run's box require),
audit logging (the seventh, 6.15), restricting every thread at once (the eighth, 7.0),
and pathname unix sockets (the ninth, 7.1).

## What seccomp holds

seccomp filters system calls by number and by scalar arguments, and a filter is inherited and never removed.
It sees a socket's family and type, `clone`'s flags, a mapping's protection bits, an ioctl's request number, and the resource-limit calls.
It can't read memory, so it never sees a path, an address, a hostname, or a string.
That's why the two layers pair: seccomp closes the gaps Landlock has by family, as each box needs
(unix sockets, UDP where no namespace holds the network, netlink, forking and executable mappings in eval's, injecting terminal input),
and Landlock does the per-hierarchy and per-port work seccomp can't see.
Neither can tell one remote host from another, so host scoping is the runtime's own check alone,
and the run's box holds the network whole, by a namespace with only loopback, never by host ([below](#namespaces)).

`fgro eval`'s filters are the built example: a kill tripwire on the calls nothing in the runtime makes,
then an allowlist from a census of what it does make, with every other call returning `ENOSYS` ([eval](eval.md#the-box)).

## Namespaces

The usual answer to "sandbox the process" is a namespace sandbox like bubblewrap:
a mount namespace with read-only binds, a network namespace with nothing in it, a pid namespace.
A mount namespace would hide or write-protect single paths inside a granted tree,
which Landlock can only approach by granting the writable subtrees one by one.
Gro holds files and the kernel's surface with Landlock and seccomp instead, and the reasons are worth stating:

- they need no privilege, no setuid helper, and no user namespace,
  which some distributions and container runtimes disable or restrict;
- a process installs them on itself, after it has done its unconfined setup and before it runs any code it doesn't trust,
  with no wrapper binary in the way;
- they inherit across `execve`, so every child is held with no cooperation from the child;
- they nest, so a child can tighten further under the parent's rules.

Their cost: both need `no_new_privs`, so a setuid helper in a child (Chromium's setuid sandbox under a browser test) can't work inside the box,
and Chromium's own user-namespace sandbox falls under the box's refusal of nested user namespaces.

The network is the one place the run's box takes a namespace, since nothing else closes Landlock's port-only rule.
Before its box goes up the run enters a private user and network namespace of its own,
keeping its user and group ids, with only a loopback interface, which it brings up,
so a test that binds and connects to `127.0.0.1` passes and nothing outside the run is reached, in or out.
Where unprivileged user namespaces are restricted (by a sysctl, by AppArmor's policy, by a seccomp filter, or with none left to make),
the box keeps the host's network and refuses every internet socket instead, loopback's included, so such a test fails there.
AppArmor's restriction is read before the namespace is made, since under it the namespace is made
and then left unusable, stranding the process; a test run with the restriction on holds that an unchecked one is unusable there.
Built as the run's steps before its box, and installed by no command yet.

## The box for runs and children

As designed, a Gro command on the binary runs in the run's box: its rules are derived from the command's closure
(the grants of the builtins it runs, [task](task.md#a-builtins-host-and-grants)) and the grant flags,
installed from the process's one thread before V8 starts, and every program the run starts inherits them.
A narrower box for each program inside the run's is designed after it, and when `fgro run` gets the run's box is open.
What's built: the rules as data, each a handle opened before the box goes up, a rule that writes opened beneath a directory the run holds open,
with every symlink on its path refused; the denylist; the steps before the box; and the type state that takes them in order,
held from inside the box by a test. Nothing derives the rules or installs the box yet. The rules:

- **Reads:** the project root and the packages linked into it, the pinned programs, the system files below,
  and in the home directory only git's global config, `~/.gitconfig` and the `~/.config/git` directory,
  with any credential or token kept there.
- **Writes:** only the command's write roots, each a rule of its own, never the project root.
  A write grant on the project, like `gen`'s or `format`'s, becomes a rule per entry,
  leaving out `.git`, `node_modules`, `.env*`, and the other dot-entries,
  and a check-mode command, like a plain `check` or `gen --check`, gets no write on the project from its grants.
  The tool directories are write roots: `.svelte-kit`, fuz_css's `.fuz/cache`,
  and the ones tools write in `node_modules`, `$app` (SvelteKit's sync), `.vite` (Vitest's optimizer),
  and `.vite-temp` (Vite's config loader), each created empty where absent before the box, for a project detected as SvelteKit or Vite,
  beside a temp directory private to the run, made before the box.
  So `.git`, the rest of `node_modules`, and under a check-mode command every source and `package.json`
  stay read-only to every process in the box at the kernel, not only to the runtime's own ops.
- **Execute:** the system's programs, the project's `node_modules` (a native binary a tool runs, like esbuild's under Vite),
  and the run's private temp directory, where test suites run scripts they write.
  The right bounds `execve` alone: a native library loads from anywhere the box reads.
  A pinned `.bin` script launches as `node` over the script's real path, so it needs no rule of its own.
  For `fgro run`, the exec roots a run declares for `spawn.program_in`, each a directory that may also be a write root,
  since a run that builds a program and then runs it, or cargo running the build scripts it compiles into its target directory, needs both on one hierarchy.
- **System files:** read rights on a baseline the binary carries as data,
  the libraries and `/etc` entries the tools need (the resolver's and certificate files among them) and devices like `/dev/null`;
  the exact set is decided with the rules.
- **Network:** the run's own namespace, whose only interface is its loopback,
  or where user namespaces are restricted the host's network with every internet socket refused ([namespaces](#namespaces)).
- **seccomp:** a denylist that classifies every call number, refusing by family with its reason (built):
  unix sockets but a stream or sequenced-packet pair connected to itself, which a child's pipes use,
  and every socket family but IPv4 and IPv6, whose TCP and UDP reach only the namespace, and in the host's network none at all;
  new namespaces of any kind, and mounts; the keyrings, System V IPC, and POSIX message queues;
  changing a file's owner, group, or extended attributes, with mode and time changes left a stated gap;
  injecting terminal input (`TIOCSTI`, `TIOCLINUX`); another process's memory and resource limits; as defense in depth, the kernel's wider surface (BPF, performance counters, `io_uring`, loading a kernel or module, opening a file by handle) and the machine's administration;
  and every number above the table's, so a call a newer kernel adds is refused until it's classified.
- **`/proc`:** the run marks itself not dumpable before its box goes up (built in the steps),
  so no program in the box reads its environment or memory through `/proc`, whatever the rules grant there.
  The programs it starts are dumpable, so rules that granted `/proc`, which gro's own suite reads,
  would let one tool read another's; what the baseline grants there is decided with the rules.
- **Resource limits:** the `core` hard limit, the only one lowered, set to 0 before the box (built in the steps),
  which every program the run starts inherits, so a crash writes no memory or environment to disk;
  a process changes only its own limits, the denylist refusing a limit set on another.

What this is designed to stop is the time-shifted write: a dependency's test running under a Node tool
plants code where the user's own tools execute it later, `.git/config`'s `fsmonitor`, a hook, a shim in `node_modules/.bin`, an editor task.
With the project read-only but for the write roots, those paths are out of reach even for code that holds the invocation's whole grant set.
What it leaves: a write into a write root that another tool then executes, the tool directories included,
which is why gen's write roots are declared, reviewed, and narrow.
An exec root that is also a write root accepts the same case for the run's own programs, so each is declared and reviewed as narrowly.
Until each program has a box of its own, one tool can write every write root the command has, so its code can plant in another's.

Two things stay coarse. The box's network is the run's own loopback,
so a command that installs, which needs the network, runs on the Node Gro,
and how a run granted hosts, and its children resolving names, would reach them from inside the box is open.
And an unconfined phase, like Gro's `publish` and `deploy`, unboxes the whole invocation, including the build it runs;
confining the build and unboxing only the pinned git and npm commands is the lean.

## The terminal

A terminal is a channel in both directions, and code in a box can still write to it.
Escape sequences can set the clipboard, change the title, disguise a link, or redraw lines the host printed,
so a dependency's output could forge a denial, a diff, or a copyable fix in the host's own format.
A Deno-style permission prompt on the same terminal is the sharpest case of this, and Gro has no prompts,
but output is the remainder, and the design treats it as such:

- confined console output to a terminal has its control characters escaped, apart from the colors the host itself emits;
  a run that needs raw terminal output (a TUI) takes a grant for it;
  a child that inherits the terminal writes to it raw, so its output is outside this escaping unless the host pipes it;
- the host prints grant diffs and hints only from its own commands (`gro check`, `gro policy`), never inside a run's output stream,
  and the copyable fix is a fixed command with no arguments, which recomputes the diff itself,
  so copying a forged one widens nothing beyond the real diff;
- a child that inherits the terminal can't inject input, by the seccomp rule above.

`fgro eval` keeps its machine channel apart already: the envelope alone goes to stdout, as JSON, and console output to stderr.
That separation is the runtime's, not the kernel's: seccomp allows `write` on every open descriptor,
so after an engine escape the envelope is the attacker's, its graph included,
and a cache over it trusts the run that wrote each entry, which no check of the files the graph names can replace
([eval](eval.md#the-envelope)).

## Resource limits

`fgro eval` caps its heap, its process memory, its module and envelope sizes, and its wall-clock time ([eval](eval.md#the-engine)), and can't fork ([eval](eval.md#the-box)).
Its timeout is cooperative: V8 is asked to stop, and a long builtin call finishes first.
`fgro run` has no limits today, and availability is outside the grant model:
a confined run can spend CPU, memory, and file descriptors, and fill its writable roots.
Designed: a heap cap, a wall clock, and output caps per run kind, the `core` limit above, and a hard deadline behind eval's cooperative stop.

## Open

- **The network for a run granted it.** The run's box reaches only its own loopback,
  so how a run granted hosts, and its children resolving names, reach outside the box is undecided.
- **A code cache for unconfined runs.** A cache entry is code to V8, so whoever writes one runs code in the next run that reads it.
  The confined process is the one that compiles, so an entry it wrote can't be trusted, and confined runs read no code cache;
  if a measurement shows one worth it, the route is a writer outside the box compiling from source itself.
  Whether runs under `-A` get a cache is undecided, and none is built.
- **Children that need `/proc`.** The run is not dumpable, but the programs it starts are,
  so rules that grant `/proc` let one tool read another's environment and memory.
- **Browser tests.** Chromium's sandboxes need a setuid helper or a user namespace, neither of which works inside the box,
  so how a browser-driven test runs under a task (a named grant that spawns it outside the box, or an unconfined phase) is open.
- **Hard links** in the read-deny overlay: refusing a confined read of any file with more than one link closes the gap
  but breaks pnpm's hard-linked `node_modules`; matching a read's inode against the denied files is the alternative. Undecided.
- **Platforms:** all of this is Linux's. macOS and Windows get their own designs, never a translation of these rules.
