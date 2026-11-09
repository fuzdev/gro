# backstop

> The kernel backstop under the runtime's own checks: what Landlock and seccomp each hold and can't,
> why not namespaces, the box designed for runs and their child processes, the terminal as a channel, and resource limits

> **status**: `fgro eval`'s box is built ([eval](eval.md#the-box) has its exact walls).
> Everything else here is designed, not built: `fgro run` has no kernel box and no resource limits today ([run](run.md)).

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
  The alternative is to grant the writable subtrees one by one, which is what the box for children does ([below](#the-box-for-runs-and-children)).
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
signal and abstract-unix-socket scopes (the sixth, 6.12, the floor `fgro eval` requires),
audit logging (the seventh, 6.15), restricting every thread at once (the eighth, 7.0),
and pathname unix sockets (the ninth, 7.1).

## What seccomp holds

seccomp filters system calls by number and by scalar arguments, and a filter is inherited and never removed.
It sees a socket's family and type, `clone`'s flags, a mapping's protection bits, an ioctl's request number, and the resource-limit calls.
It can't read memory, so it never sees a path, an address, a hostname, or a string.
That's why the two layers pair: seccomp closes the gaps Landlock has by family
(unix sockets, UDP, netlink, forking, executable mappings, injecting terminal input),
and Landlock does the per-hierarchy and per-port work seccomp can't see.
Neither can tell one remote host from another, so host scoping is the runtime's own check alone,
and a child process under a net grant is held to ports, not hosts.

`fgro eval`'s filters are the built example: a kill tripwire on the calls nothing in the runtime makes,
then an allowlist from a census of what it does make, with every other call returning `ENOSYS` ([eval](eval.md#the-box)).

## Why not namespaces

The usual answer to "sandbox the process" is a namespace sandbox like bubblewrap:
a mount namespace with read-only binds, a network namespace with nothing in it, a pid namespace.
Namespaces would close two things this design leaves coarse,
hiding or write-protecting single paths inside a granted tree, and the port-only network rules.
Gro uses Landlock and seccomp instead, and the reasons are worth stating:

- they need no privilege, no setuid helper, and no user namespace,
  which some distributions and container runtimes disable or restrict;
- a process installs them on itself, after it has done its unconfined setup and before it runs any code it doesn't trust,
  with no wrapper binary in the way;
- they inherit across `execve`, so every child is held with no cooperation from the child;
- they nest, so a child can tighten further under the parent's rules.

Their cost: both need `no_new_privs`, so a setuid helper in a child (Chromium's setuid sandbox under a browser test) can't work inside the box,
and Chromium's own user-namespace sandbox falls under the no-new-namespaces rule.

A network namespace for children is the one namespace worth taking where unprivileged user namespaces are available,
since nothing else closes the port-only rule or lets a child resolve names through the runtime only.
Whether to take it, and what to do where it's unavailable, is open.

## The box for runs and children

As designed, the kernel rules for a `gro run` or `gro task` invocation are derived from its grant set and installed before V8 starts,
and every child process inherits them. The rules:

- **Reads:** the project root and the packages linked into it.
- **Writes:** only the declared write roots, each a rule of its own (`dist/`, `build/`, `.gro/`, `.svelte-kit/`, `$TMPDIR`,
  and each spawned tool's own cache directory, named one by one, never all of `~/.cache`, which holds other tools' executable caches),
  never the project root. So `.git`, `node_modules`, `package.json`, the policy file, and editor and agent configuration stay read-only to every child at the kernel,
  not only to the runtime's own ops; a tool whose cache defaults to a `node_modules` subdirectory (Vite's `.vite`) is pointed at its cache directory instead.
- **Execute:** the pinned programs, their interpreters and libraries, and a native binary a tool runs from `node_modules` (esbuild's, under Vite).
- **System files:** read rights on the libraries and `/etc` entries the pinned programs need (the resolver's and certificate files among them), and `/dev/null`;
  the exact set is per tool.
- **Network:** TCP connect on the granted ports; bind only on a serve grant's declared port.
- **seccomp:** unix sockets, UDP, and netlink denied unless a grant names them (a socket pair for a child's own pipes is allowed),
  no forking into new namespaces, no injecting terminal input (`TIOCSTI`, `TIOCLINUX`),
  and no changing resource limits (seccomp sees whether a new limit is passed, not its value), with the child limits set as hard limits.
- **`/proc`:** no rule by default. If a tool needs one, that child installs a further Landlock layer before it runs,
  so the runtime's own process falls outside the child's domain and its environment and memory stay unreadable.
- **Resource limits** on children: open files, core dumps, and process count, cheap and inherited.

What this is designed to stop is the time-shifted write: a dependency's test running under a Node tool
plants code where the user's own tools execute it later, `.git/config`'s `fsmonitor`, a hook, a shim in `node_modules/.bin`, an editor task.
With the project root read-only to children, those paths are out of reach even for code that holds the invocation's whole grant set.
What it leaves: a write into a declared write root that another tool then executes, which is why gen's write roots are declared, reviewed, and narrow.

Two things stay coarse. A child under a net grant reaches any host on its ports,
and name resolution for children has no good answer without a network namespace
(a loopback proxy doesn't close it, since allowing the proxy's port allows any remote host on that port).
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
so after an engine escape the envelope is as trustworthy as the engine, and a caller that caches on it re-hashes what it reads.

## Resource limits

`fgro eval` caps its heap, its process memory, its module and envelope sizes, and its wall-clock time ([eval](eval.md#the-engine)), and can't fork ([eval](eval.md#the-box)).
Its timeout is cooperative: V8 is asked to stop, and a long builtin call finishes first.
`fgro run` has no limits today, and availability is outside the grant model:
a confined run can spend CPU, memory, and file descriptors, and fill its writable roots.
Designed: a heap cap, a wall clock, and output caps per run kind, the child limits above, and a hard deadline behind eval's cooperative stop.

## Open

- **Name resolution for children** under a net grant, and whether a network namespace is taken where available.
- **The code cache's writer.** A cache entry is code to V8, so whoever writes one runs code in the next run that reads it.
  The confined process is the one that compiles, so either no run reads an entry written by a run holding less than it does,
  or a writer outside the box compiles from source itself. Undecided, and the cache isn't built until it is.
- **Children that need `/proc`**, and whether the nested layer costs anything measurable.
- **Browser tests.** Chromium's sandboxes need a setuid helper or a user namespace, neither of which works inside the box,
  so how a browser-driven test runs under a task (a named grant that spawns it outside the box, or an unconfined phase) is open.
- **Hard links** in the read-deny overlay: refusing a confined read of any file with more than one link closes the gap
  but breaks pnpm's hard-linked `node_modules`; matching a read's inode against the denied files is the alternative. Undecided.
- **Platforms:** all of this is Linux's. macOS and Windows get their own designs, never a translation of these rules.
