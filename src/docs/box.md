# box

> The box, the kernel layer under the runtime's own checks: what Landlock and seccomp each hold and can't,
> the one namespace the box takes, the box a command of Gro's runs in on the binary and the one designed for each child, the terminal as a channel, and resource limits

> **status**: `fgro eval`'s box is built ([eval](eval.md#the-box) has its exact walls),
> and so is the run's box, which every command of Gro's the binary runs starts in, `fgro check` and `fgro gen` among them
> ([below](#what-a-commands-box-admits) has its exact walls).
> `fgro run` has no kernel box and no resource limits yet ([run](run.md)): its box is designed, to come after.
> A narrower box for each tool inside the run's is designed, to land after it, and the first release waits for it.
> The read-deny overlay and the rest here not marked built are designed too.

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
  The alternative is to grant the writable subtrees one by one, which is what the run's box does ([below](#what-a-commands-box-admits)).
- **Metadata stays readable everywhere**: `stat`, a symlink's target, and so whether a path exists,
  and an inotify watch, which tells the names of entries made, changed, and removed in any directory the user can reach, one the box can't list among them.
- **A hard link is the file.** One made outside the box (a dependency's install script, an unconfined phase) inside a granted hierarchy
  reads like any other file under whatever name it has, so a name-based deny (`.env*`, credential files) can't see it;
  a confined process can't link in a file from a hierarchy its rules don't let it write, since linking needs the `Refer` right on both sides.
- **Its network rules are TCP ports**, bind and connect, with no addresses, no hostnames, and no UDP.
- **Pathname unix sockets** aren't gated until the ninth ABI (Linux 7.1); abstract ones are scoped from the sixth.
- **It only tightens.** A thread's domain can gain layers but never lose one, and a child inherits it;
  the box goes up from the one thread before V8 starts any, so an invocation with an unconfined phase is unboxed from start to finish.

The ABIs the design leans on, with the kernel each arrived in:
filesystem rules (the first, Linux 5.13), linking and renaming across hierarchies (the second, 5.19), truncation (the third, 6.2),
TCP bind and connect by port (the fourth, 6.7), device ioctls (the fifth, 6.10),
signal and abstract-unix-socket scopes (the sixth, 6.12, the floor `fgro eval` and the run's box require; below it a command of Gro's [falls back](#how-it-goes-up)),
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
Built, and taken by every command of Gro's the binary runs, which says which form it got when it's the second:
a `note:` line, once the box is up, names why there's no namespace and that a test binding `127.0.0.1` fails.

## The box for runs and children

Every command of Gro's that the binary runs, like `fgro check` or `fgro gen`, starts in the run's box (built):
its rules are derived from the command's closure (the grants of the builtins it runs, [task](task.md#a-builtins-host-and-grants)) and the grant flags,
installed from the process's one thread before V8 starts, and every program the run starts inherits them.
A command the binary leaves to Node runs on the Node Gro, which confines nothing ([fallback](glossary.md#fallback)).
`fgro run` has no box yet: its box, derived from the run's grant flags alone, is designed to come after.
A narrower box for each program inside the run's is designed too ([below](#a-box-for-each-tool)).

### What a command's box admits

The rules come from one plan, derived from the command's closure, the grant flags, and what the run reads from the disk before the box:
the project's top-level entries, its `node_modules`, which system directories are real ones, the packages linked into the project,
where `node` and the granted programs are, and what git reads outside the project.
Each rule is a handle opened before the box goes up: by its real path, which can read and execute only,
or beneath a directory the run holds open, with every symlink on the path refused, the only way a rule writes.
A rule whose path is absent is skipped and recorded when it's optional, like a device a system lacks, and refuses the run when it's required;
anything else that keeps a rule from opening, like a symlink on its path, refuses the run either way.

- **Reads:** the project, all of it (`.git` and `.env*` included), and the packages linked into it from outside, each by its real path ([link closure](glossary.md#link-closure)),
  the system files below, and `/proc`, as its own item below says.
  A command that may start a program also reads each program it pins, by its real file (for a `node_modules/.bin` script, the `node` it runs under),
  and what git reads outside the project, each that exists, by its real path:
  - git's global config, `~/.gitconfig` and `config` in git's directory, `$XDG_CONFIG_HOME/git` or `~/.config/git`,
    or, when `GIT_CONFIG_GLOBAL` is set, the file it names in their place;
    `ignore` and `attributes` in git's directory;
    and the files those configs include (`include.path`, and `includeIf.<condition>.path` whatever its condition)
    and name as `core.excludesFile` and `core.attributesFile`, read from the configs as git reads them.
    A dotfile manager often links these, so each is read where it really is;
    nothing inside the project's repository, or reached through it, is granted this way, so the project can't plant a path here.
    Git's credential store (`~/.git-credentials`, or `credentials` in git's directory) stays unreadable unless your own git config names it as one of those files,
    and any credential or token kept in the config files themselves is readable.
  - git's own directories when they sit outside the project: a linked worktree's git directory and the common one it shares,
    a submodule's git directory, or the `.git` of a repository the project sits inside, the worktree's and the submodule's each held to lead back to this checkout.
    A `.git` at the home directory or above, like a dotfiles repository's, gets no rule, and git run inside the box fails there.

  A git config the box can't read as git does, an include nested past git's limit, a config naming a directory or a path the box can't resolve (`~user/`),
  or a `.git` file that isn't a linked worktree's or a submodule's leading back to this checkout refuses a command that may start a program before the box, exit 2,
  since a git that couldn't read them would fail every command, or drop a global ignore unseen.
  Nothing else under the home directory is readable but the programs the run pins (`node`'s file among them, where a version manager like fnm or nvm installed it there),
  what a grant flag names, the project's own trees wherever they sit (the project, its linked packages, a linked `node_modules`),
  and the run's temp directory where `TMPDIR` puts it there.
- **Writes on the project:** a command whose closure writes the project, like `gen`, `format`, or `sync`, gets a rule per top-level entry, never the project root.
  Each existing directory and regular file is one, but `.git`, `node_modules`, and every other dot-entry (`.env*`, `.github`, and `.gro` among them),
  every symlink (an `AGENTS.md` that links `CLAUDE.md` stays read-only), a regular file with more than one link, and a FIFO, socket, or device.
  A top-level entry the command would make, like a genfile's new output beside `src`, has no rule, so it's refused.
  A check-mode command, like a plain `check`, `gen --check`, or `format --check`, gets no write on the project from its grants,
  and neither do `test`, `typecheck`, and `lint`.
- **Tool directories:** the directories the project's tools write on their own,
  each a write root when the command's closure grants a tool from the project's `node_modules/.bin` and the project declares the dependency that writes it:
  `.svelte-kit` and `node_modules/$app` (SvelteKit's sync) for `@sveltejs/kit`,
  `node_modules/.vite` (Vite's optimizer and Vitest's) and `node_modules/.vite-temp` (Vite's config loader) for `vite` or `vitest`,
  and `.fuz/cache`, which fuz_css's plugin writes as Vitest transforms, when the closure grants Vitest and the project declares `@fuzdev/fuz_css`, with `.fuz` itself read-only.
  Each is made as a real directory where it's absent before the box, every directory on its path opened with symlinks refused,
  and one already there as a symlink or anything but a directory refuses the run, exit 2, so a link planted by an earlier run can't send it elsewhere;
  one that can't be made fails the run, exit 1.
  So a plain `check` writes the tool directories and nothing else in the project, and `format --check`, which starts no tool, writes nothing at all.
  Vitest's module cache under `node_modules` is never one: it holds transformed code that later Vitest runs load unverified,
  so a test that could write it could plant code for a run outside any box.
  `gro test` turns the cache off on either runtime ([test](test.md#vitests-module-cache)), and in the box a test can't write it either.
- **The run's temp directory:** a directory private to the run, made in the OS's temporary directory before the box
  for a command that may start a program or that `--allow-temp` grants, which it may read, write, and execute in.
  `TMPDIR` names it for the run and its programs, and so do `TMP` and `TEMP` where the process had them set,
  and `XDG_DATA_HOME`, `XDG_CACHE_HOME`, and `XDG_STATE_HOME` name directories inside it,
  so a tool that keeps state under those (Vitest keeps a token there, and won't start without one) gets a fresh, private place,
  and nothing under the home directory is read or written for it.
  A run with no private directory keeps those variables as they were.
  An OS temporary directory inside the project, by its path or by its identity (a bind mount counts), refuses such a run, exit 2, before anything is made,
  since the run's tools would write an entry there that later commands read; the refusal names the variable that set it, or says it's the default.
  The run holds its directory by a lock, and a later start removes one no running process holds, never following a link inside it
  and never inside the project, which keeps leftovers from piling up and is no wall.
- **Execute:** for a command that may start a program, the system's programs, each pinned program's file (`node`'s among them),
  the run's temp directory, where test suites run scripts they write, the project's `node_modules`, and the linked packages, each on its real tree,
  for a native binary a tool may run, like esbuild's under Vite, beside the directories `--allow-run-in=` names.
  The rest of the project isn't executable, so a tool runs a script there only through its interpreter.
  The right bounds `execve` alone: a native library loads from anywhere the box reads.
  A pinned `.bin` script runs as `node`'s argument, by the script's real path, so it needs no rule of its own and nothing runs `/usr/bin/env`.
- **System files:** `/usr` read and executed and `/etc` read, `/bin`, `/sbin`, `/lib`, `/lib32`, and `/lib64` only where they're real directories rather than links into `/usr`,
  and the devices `/dev/null` (read and written), `/dev/zero`, `/dev/urandom`, and `/dev/random`.
  The binary carries the set as data. No rule names the binary's own file.
- **The grant flags** widen the box as they widen the realm:
  `--allow-read=` reads a directory by its real path, `--allow-write=` reads and writes one (opened beneath `/` by its real path, with every symlink refused),
  `--allow-run-in=` reads and executes one, `--allow-run=` pins a program whose file is read and executed, and `--allow-temp` grants the run's temp directory.
  A write flag naming the project root, by any spelling, adds the same rule per top-level entry a write on the project gets.
  One naming an entry the box leaves out, like `--allow-write=.git`, is honored, the command line's own choice.
  One naming a directory that holds the project root, like `--allow-write=..`, is refused, exit 2,
  since a rule there would cover `.git` and every entry the box leaves out, and Landlock can't carve them back out.
- **A `node_modules` that's a symlink**, as agent worktrees and scratch copies make, is followed only to another checkout's:
  a directory named `node_modules` outside the project, neither inside it nor holding it.
  It's read there on its real path (and executed, for a command that may start a program), and one `note:` line names where it leads.
  Gro's own code reads through it too, as a read-only root of the run's `fs` on the same resolution the box follows,
  so a read the box allows there isn't refused as a symlink leaving the project.
  A package there resolves its imports within that `node_modules`, and nothing of the checkout holding it is read,
  its `package.json` included ([resolver](resolver.md#departures-from-node)).
  No tool directory is made there, and nothing there is written, unless a write flag names it,
  since that checkout's own unconfined commands run what a test writes there, so a tool that needs one fails without the flag.
  A link to any other directory, like `~/.ssh` or `.git`, is refused, exit 2.
- **Network:** the run's own namespace, whose only interface is its loopback,
  or where user namespaces are restricted the host's network with every internet socket refused ([namespaces](#namespaces)).
- **seccomp:** a denylist that classifies every call number, refusing by family with its reason:
  unix sockets but a stream or sequenced-packet pair connected to itself, which a child's pipes use,
  and every socket family but IPv4 and IPv6, whose TCP and UDP reach only the namespace, and in the host's network none at all;
  new namespaces of any kind, and mounts; the keyrings, System V IPC, and POSIX message queues;
  changing a file's owner, group, or extended attributes, with mode and time changes and attribute flags set by `ioctl` or `file_setattr` left a stated gap;
  injecting terminal input (`TIOCSTI`, `TIOCLINUX`); another process's memory (`ptrace`, `process_vm_*`) and resource limits, with its scheduling left a stated gap; as defense in depth, the kernel's wider surface (BPF, performance counters, `io_uring`, loading a kernel or module, opening a file by handle) and the machine's administration;
  and every number above the table's, so a call a newer kernel adds is refused until it's classified.
- **`/proc`:** readable, whole and read-only, so Node's process APIs (like `process.memoryUsage()`) behave as they do on Node.
  A rule can't name `/proc/self` for a child, since the handle names whichever process opened it, so the choice is all of `/proc` or none.
  Boxed code sees the command line and status of every process your user can see, as any unconfined `dev` does,
  but the environment, memory, and working directory of a process outside the box stay refused, by Landlock's restriction on tracing a process outside its domain.
  The run marks itself not dumpable before its box goes up, so no program in the box reads the run's own environment or memory either.
  The programs it starts are dumpable and share the run's box, so until each has a box of its own, one tool reads another's environment and working directory,
  and its memory too where Yama's `ptrace_scope` is 0 (at 1, only its own descendants'; at 2 or 3, none), since Yama restricts what attaching needs, which reading the environment doesn't.
- **Resource limits:** the `core` hard limit, the only one lowered, set to 0 before the box,
  which every program the run starts inherits, so a crash writes no memory or environment to disk,
  a guardrail against that accident and not a wall, since deliberate code writes what it holds into a write root;
  a process changes only its own limits, the denylist refusing a limit set on another.
- **The environment:** the run's programs see the run's own, as on Node, but for the temp and XDG variables above
  and the vendored `deno_core`'s diagnostic variables, which the run removes ([run](run.md#usage)).

The realm and the box are derived from the same run, and the box's plan feeds the realm one root back, the linked `node_modules` above.
The realm's roots are directories, and its write on the project is the project root;
the box is narrower on the project and wider for the tools.
So a write the realm admits and the box refuses, like a genfile's new top-level output or a write under `.git`, fails as an `FsError` of kind `permission_denied`,
and a tool's own write fails with the kernel's `EACCES`.
The `FsError`'s cause gains what the box lets the command write and what stays read-only, as in

```
Permission denied (os error 13): the run's kernel box lets fgro gen write only to the top-level entries the project had as it started (but .git, node_modules, the dot-entries, symlinks) and a temporary directory of the run's; .git, node_modules, the other dot-entries, and a new top-level entry stay read-only
```

when the path the write reaches lies under none of the box's write rules, resolved within the run's roots as Landlock checks each op:
for a `remove`, either end of a `rename`, a `symlink`, or a `mkdir`, the directory the entry sits in, which is what the kernel checks,
so removing or renaming a top-level entry names the box, since the project's root itself isn't writable;
and for a file written or a copy's destination, the whole path, a final symlink followed.
A mode or time change never gains the cause, since Landlock checks neither,
and a refusal of a path the box writes, like a file's own mode refusing it, gains nothing.
Two kinds of refusal the box makes print no cause: a write through a dangling symlink, resolved only to its own parent,
and a recursive `mkdir` or copy through a symlink on the way.
A run that exits nonzero after a tool it started ended other than by exiting 0, not counting one the run signaled itself, ends with a `note:` line naming the box's network and writes, as in

```
note: fgro gen's tools ran in the run's kernel box, which lets them reach no network but the run's own loopback and write only to the top-level entries the project had as it started (but .git, node_modules, the dot-entries, symlinks) and a temporary directory of the run's: a tool that hits a wall fails with EACCES, EPERM, or ENOSYS, and --allow-read=, --allow-write=, and --allow-run-in= widen the box
```

Both are built from the rules the box installed: only tool and flag directories are named, and in the host's network the note's clause reads "reach no network".
Both are review signals, which the run's code can print as well.

### How it goes up

The binary takes the box's steps on its one thread, before V8 or any other thread starts, in an order that keeps a fallback an unchanged process:

1. The pre-pass reads the command's closure, the grant flags, and the project, and holds no descriptor when it returns.
   Every fallback the command line or the project's files decide runs the Node Gro here, so Node gets the process the Node Gro would have,
   and what it refuses, like `-A` before the command word, exits 2.
2. It refuses what can't run boxed, before anything changes:
   a binary built without its snapshot, which would read its own sources from its build tree, exit 1 ([run](run.md#errors-and-exit-codes)),
   and `-A` after the command word, whose unconfined spelling is the Node Gro, exit 2.
3. It probes the kernel and the standard streams, changing nothing.
   Where the kernel's Landlock is missing, off, or below its sixth ABI, or a seccomp filter the process inherited refuses its calls, there's no box, so the command runs on the Node Gro after a `note:` line saying so, a fallback the host decides,
   which `GRO_FALLBACK=command` refuses, exit 2 ([authority](authority.md#what-the-first-release-is-for)),
   as it's refused, exit 2, when the project installed no Node Gro to run.
   A socket on stdin, stdout, or stderr that isn't a connected unix stream is refused, exit 2, since a box can't hold where it leads.
4. Then the first change: it closes every descriptor it inherited above stderr, and from here nothing falls back.
   It enters the network namespace, then drops its capabilities, marks itself not dumpable, lowers the `core` limit, and raises its open-files soft limit, as Node does.
5. It opens the OS's temporary directory by its real path and removes the stale private directories there, unless it lies in the project.
6. It resolves once what the run and its box both use: the directories the grant flags list, the programs it pins, `node` among them, and the working directory.
7. It derives the box's plan from that resolution and the disk's facts, read once, and prints the plan's note.
   The plan refuses, exit 2, a write flag naming a directory that holds the project root;
   a `node` on `PATH` that's a script, like an asdf or nodenv shim, which reads its manager's tree under the home directory, naming the shim and the fix,
   unless an `--allow-run-in=` names the directory it's in;
   a `node_modules` symlink to anything but another checkout's `node_modules`;
   and, for a command that may start a program, git's files it can't read.
8. It makes its private directory, where it has one, refusing an OS temporary directory inside the project, exit 2, then the tool directories.
9. It sets the run's environment, the temp and XDG variables pointed into the private directory and the vendored `deno_core`'s diagnostic variables removed,
   and puts `SIGCHLD` back at its default, so each tool it starts is reaped by its own handle.
10. It opens the rules from the same plan, so a rule and the pin it admits can't name different files,
    installs Landlock and the denylist, prints the network's `note:` line when there's no namespace, and starts V8 on a thread of its own.

From the first change on, a path planted where the box opens a rule or makes a tool directory refuses the run, exit 2, as the plan's refusals do,
and a step that fails ends the run, exit 1.
A program that preloads a library starting a thread (`LD_PRELOAD`) can leave the process with more than one, which the box refuses, naming the count, exit 1.

### When a command hits a wall

A tool that meets the box fails with the kernel's own error:
`EACCES` for a path its rules leave out, `EPERM` for a signal to a process outside the run or a change of a file's owner,
and `ENOSYS` for a call family the box refuses.
Gro's own code fails with a `permission_denied` `FsError` whose cause names where the box lets the command write,
and a run that fails after a tool did ends with a `note:` line naming the box's network and writes ([above](#what-a-commands-box-admits)).
The grant flags, right after the command word, widen the box for one run:

- a read outside the project, like a tool's config or cache under the home directory, takes `--allow-read=<dir>`
- a write outside the write roots takes `--allow-write=<dir>`, naming the directory itself, never one that holds the project root
- a program run from a directory outside `node_modules` and the system's takes `--allow-run-in=<dir>`

No flag gives a new top-level entry a rule, since `--allow-write=.` adds one for each entry the project has as the run starts,
so make the entry before the run.
Nothing widens the network past the run's own loopback.
What no flag reaches runs on the Node Gro, `node_modules/.bin/gro <command>`, which confines nothing.

### What it holds, and what it leaves

The box is a [boundary](glossary.md#boundary), the kernel's, against the project's code and tests, the tools and their children, and an engine exploit,
and opening each rule and tool directory with every symlink refused is part of it, against a link planted by an earlier run.
What defeats it: a write into a write root that a later unconfined command runs, the mode and time gap, a hard link made outside the run,
a secret the rules let it read, like `.env*` in the project,
a `.git` or `node_modules` nested inside an included directory, which Landlock can't carve out, what the run prints, and a kernel bug.
Every `note:` line and refusal message it prints is a [review signal](glossary.md#review-signal) the run's code can also print.
Inside the box Gro's per-builtin grants stay [guardrails](glossary.md#guardrail).

What it stops is the time-shifted write: a dependency's test running under a Node tool
planting code where the user's own tools execute it later, `.git/config`'s `fsmonitor`, a hook, a shim in `node_modules/.bin`, an editor task.
With the project read-only but for the write roots, those paths are out of reach even for code that holds the invocation's whole grant set.
What the project's own files already run, the box bounds as it bounds the tools:
the config and genfiles, and the commands a repository's own config names, like `core.fsmonitor`, which git runs on `status`, `ls-files`, and `diff`.
What it leaves: a write into a write root that another tool then executes, the tool directories included
(`node_modules/.vite` holds the optimized dependencies a later `dev` loads),
and under `gen`, `format`, or `sync` any included entry the next unconfined command runs, like `src`, `package.json`, or `vite.config.ts`,
which is why a check-mode command writes none of them.
An exec root that is also a write root accepts the same case for the run's own programs, so each is declared and reviewed as narrowly.
The tools share the one box, so until each has its own, one tool can write every write root the command has, and its code can plant in another's,
and can read another's environment through `/proc`, and its memory where Yama allows ([above](#what-a-commands-box-admits)).

### A box for each tool

Designed, to land after the run's box, and the first release waits for it: each program the run starts runs in a box of its own, nested in the run's.
Landlock restricts the thread that installs its rules, so the run starts each program from a short-lived thread that first restricts itself with that program's rules
and then spawns it as today, and the program inherits the narrower box; nothing re-executes the binary.
Each program gets its own temp directory.
A program the grants table grants writes only the directories the table names for it:
the tools that write on their own, like SvelteKit's sync and Vitest, name their own write roots, and the ones that don't, like tsc, ESLint, and git, name none.
Vitest is held to SvelteKit's generated modules for its dev server and its own cache in `node_modules/.vite`, with its config loaded by Vite's runner loader,
which needs no write of its own.
A program a grant flag names gets no write on the project, and a flag naming a write root for one program waits behind the release.
Nested, the boxes also keep a tool from signalling Gro and from reading a sibling through `/proc`.
That closes a test or an ESLint plugin planting in what `dev` and `build` run.
It leaves the project's own config, which runs in every tool and which `dev` runs unconfined,
and a write into a tool's own directory that a dev server already running picks up.

### What stays coarse

The box's network is the run's own loopback,
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
A command of Gro's lowers its `core` hard limit to 0 in its box ([above](#what-a-commands-box-admits)) and caps nothing else,
`fgro run` has no limits today, and availability is outside the grant model:
a confined run can spend CPU, memory, and file descriptors, and fill its writable roots.
Designed: a heap cap, a wall clock, and output caps per run kind, the `core` limit for `fgro run`, and a hard deadline behind eval's cooperative stop.

## Open

- **The network for a run granted it.** The run's box reaches only its own loopback,
  so how a run granted hosts, and its children resolving names, reach outside the box is undecided.
- **A code cache for unconfined runs.** A cache entry is code to V8, so whoever writes one runs code in the next run that reads it.
  The confined process is the one that compiles, so an entry it wrote can't be trusted, and confined runs read no code cache;
  if a measurement shows one worth it, the route is a writer outside the box compiling from source itself.
  Whether runs under `-A` get a cache is undecided, and none is built.
- **Secrets in the project.** The run's box reads the whole project, so `.env*` and any credential kept in the tree stay readable to every process in it;
  the read-deny overlay that would refuse them is designed, not scheduled ([authority](authority.md#what-the-first-release-is-for)).
- **Browser tests.** Chromium's sandboxes need a setuid helper or a user namespace, neither of which works inside the box,
  so how a browser-driven test runs under a task (a named grant that spawns it outside the box, or an unconfined phase) is open.
- **Hard links** in the read-deny overlay: refusing a confined read of any file with more than one link closes the gap
  but breaks pnpm's hard-linked `node_modules`; matching a read's inode against the denied files is the alternative. Undecided.
- **Platforms:** all of this is Linux's. macOS and Windows get their own designs, never a translation of these rules.
