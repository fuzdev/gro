# box

> The box, the kernel layer under the runtime's own checks: what Landlock and seccomp each hold and can't,
> the one namespace the box takes, the box a command of Gro's runs in on the binary and the one each tool runs in inside it, the terminal as a channel, and resource limits

> **status**: `fgro eval`'s box is built ([eval](eval.md#the-box) has its exact walls),
> and so is the run's box, which every command of Gro's the binary runs starts in, `fgro check` and `fgro gen` among them
> ([below](#what-a-commands-box-admits) has its exact walls).
> `fgro run` has no kernel box and no resource limits yet ([run](run.md)): its box is designed, to come after.
> Each tool the run starts runs in a narrower box of its own inside the run's,
> writing only its own directories ([below](#a-box-for-each-tool)).
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

Their cost: both need `no_new_privs`, so a setuid or setcap program, like `sudo` or `ping`, runs without its privileges inside the box,
a setuid helper in a child (Chromium's setuid sandbox under a browser test) can't work there,
and Chromium's own user-namespace sandbox falls under the box's refusal of nested user namespaces.

The network is the one place the run's box takes a namespace, since nothing else closes Landlock's port-only rule.
Before its box goes up the run enters a private user and network namespace of its own,
keeping its user and group ids, with only a loopback interface, which it brings up,
so a test that binds and connects to `127.0.0.1` passes and nothing outside the run is reached, in or out.
Where unprivileged user namespaces are restricted (by a sysctl, by AppArmor's policy, by a seccomp filter, or with none left to make),
the box keeps the host's network and refuses every internet socket instead, loopback's included, so such a test fails there.
AppArmor's restriction is read before the namespace is made, since under it the namespace is made
and then left unusable, stranding the process.
Built for every command of Gro's the binary runs.
Without the namespace, a `note:` line once the box is up says why and that a test binding `127.0.0.1` fails.

## The box for runs and children

Every command of Gro's that the binary runs, like `fgro check` or `fgro gen`, starts in the run's box (built):
its rules are derived from the command's closure (the grants of the builtins it runs, [task](task.md#a-builtins-host-and-grants)) and the grant flags,
installed from the process's one thread before V8 starts, and every program the run starts inherits them.
A command the binary leaves to Node runs on the Node Gro, which confines nothing ([fallback](glossary.md#fallback)).
`fgro run` has no box yet: its box, derived from the run's grant flags alone, is designed to come after.
Each program the run starts runs in a narrower box of its own inside the run's ([below](#a-box-for-each-tool)).

### What a command's box admits

In short: the project is read-only to every process in the run but for the write roots that follow,
and outside it little under the home directory is readable (the reads below list what is).
Gro's own code writes the project's top-level entries only under `gen`, `format`, and `sync`, never `.git`, `node_modules`, or another dot-entry.
Each tool writes only its own tool directories, its own temporary directory, and what `--allow-run-write=` names for it.
The network is the run's own loopback (or no internet socket at all where user namespaces are restricted), and the grant flags widen the rest as the list below says.

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
  Each existing directory and regular file gets one, except `.git`, `node_modules`, every other dot-entry (`.env*`, `.github`, and `.gro` among them),
  every symlink (an `AGENTS.md` that links `CLAUDE.md` stays read-only), a regular file with more than one link, and a FIFO, socket, or device.
  A top-level entry the command would make, like a genfile's new output beside `src`, has no rule, so it's refused.
  A check-mode command, like a plain `check`, `gen --check`, or `format --check`, gets no write on the project from its grants,
  and neither do `test`, `typecheck`, and `lint`.
- **Tool directories:** the directories the project's tools write on their own,
  each a write root of the tools whose grant in the grants table names it (their `write`),
  when the command's closure grants such a tool and the project declares a dependency that calls for it:

  | directory                   | what it holds                                                  | written by                         | called for by      |
  | --------------------------- | -------------------------------------------------------------- | ---------------------------------- | ------------------ |
  | `.svelte-kit`               | all of what `svelte-kit sync` writes, which `vite build` reads | `svelte-kit`                       | `@sveltejs/kit`    |
  | `.svelte-kit/generated/dev` | the modules SvelteKit generates for a dev server               | svelte-check, Vitest               | `@sveltejs/kit`    |
  | `.svelte-kit/types`         | the route types                                                | Vitest                             | `@sveltejs/kit`    |
  | `node_modules/$app`         | the tsconfig and types SvelteKit generates                     | `svelte-kit` alone                 | `@sveltejs/kit`    |
  | `node_modules/.vite-temp`   | the bundled config Vite's config loader writes                 | `svelte-kit`, svelte-check, Vitest | `vite` or `vitest` |
  | `node_modules/.vite/vitest` | Vitest's own cache                                             | Vitest                             | `vitest`           |

  `node_modules/$app` is `svelte-kit`'s alone because a later typecheck, `dev`, `build`, and the editor trust what's there,
  and a file planted beside the generated ones, like a `package.json` whose `exports` redirect `$app/tsconfig`, survives every sync;
  `gro test` syncs first ([test](test.md#sveltekits-files-synced-first)), so Vitest finds nothing there to write.
  No tool writes `node_modules/.vite` beside Vitest's cache, where a later `dev` loads its optimized dependencies,
  nor fuz_css's `.fuz/cache`, whose entries a later `dev` or `build` trusts, though Vitest's dev server writes it as it scans the project:
  fuz_css skips a refused cache write quietly.
  Vitest's module cache, `node_modules/.vitest-cache`, is never a tool directory ([test](test.md#vitests-module-cache)).

  No tool makes a symlink or FIFO in a tool directory, even its own, or moves or links an entry in from another directory,
  which could bring a symlink from its own temporary directory or a directory `--allow-run-write=` names.
  The same holds, for Gro's own code and every tool, in a directory `--allow-write=` or `--allow-run-write=` names
  that is a tool directory (made or not) or a linked `node_modules`'s target, or that lies in or holds one.
  Every other directory either flag names keeps those rights, like `src` for `eslint --fix` or a snapshot directory for `test -u`.
  Otherwise a later unconfined tool that rewrites its files there by name, as SvelteKit's `vite dev` rewrites its generated files,
  would follow a planted link out of the project or block reading a FIFO.

  Each is made as a real directory where it's absent before the box, every directory on its path opened with symlinks refused,
  and one already there as a symlink or anything but a directory refuses the run, exit 2, so a link planted by an earlier run can't send it elsewhere;
  one that can't be made fails the run, exit 1.
  So a plain `check` writes the tool directories and nothing else in the project, and `format --check`, which starts no tool, writes nothing at all.
  A step the command line turns off takes its tool out of the closure, so `check --no-test` grants no Vitest, and with it none of the directories only Vitest writes.
- **The run's temp directory:** a directory private to the run, made in the OS's temporary directory before the box
  for a command that may start a program or that `--allow-temp` grants, which it may read, write, and execute in.
  `TMPDIR` names it for the run, and so do `TMP` and `TEMP` where the process had them set,
  and `XDG_DATA_HOME`, `XDG_CACHE_HOME`, and `XDG_STATE_HOME` name directories inside it.
  Each program the run starts gets a directory of its own inside it, which its `TMPDIR`, its `TMP` and `TEMP` where its environment has them, and its XDG variables name,
  over whatever its environment asked for, and no program reads the run's or another program's,
  so a tool that keeps state under those (Vitest keeps a token there, and won't start without one) gets a fresh, private place,
  and nothing under the home directory is read or written for it, so a tool that caches there or in its own directory starts cold every run.
  A file Gro's own code makes in the run's temp directory, like one `fs.temp_dir` makes, is unreadable to every tool.
  A run with no private directory keeps those variables as they were.
  An OS temporary directory inside the project, by its path or by its identity (a bind mount counts), refuses such a run, exit 2, before anything is made,
  since the run's tools would write an entry there that later commands read; the refusal names the variable that set it, or says it's the default.
  For a command that may start a program, a rule a program's box takes holding the OS temporary directory, like `--allow-read=/tmp`, `--allow-run-in=/tmp`, `--allow-read=/`, or `--allow-run-write=vitest:/tmp` (never `--allow-write=`, whose directories no program's box takes),
  refuses it too, exit 2, before anything is made, since through that rule each program could read the whole of the run's temp directory, the others' temporary directories in it included;
  the refusal names the flag or rule, and says to narrow the flag or to point the variable that set it elsewhere.
  The run holds its directory by a lock, and a later start removes one no running process holds, never following a link inside it
  and never inside the project, which keeps leftovers from piling up and is no wall.
- **Execute:** for a command that may start a program, the system's programs, each pinned program's file (`node`'s among them),
  each program's own temporary directory, where test suites run scripts they write, the project's `node_modules`, and the linked packages, each on its real tree,
  for a native binary a tool may run, like esbuild's under Vite, beside the directories `--allow-run-in=` names.
  The rest of the project isn't executable, so a script there run by its path fails with `EACCES`, and a tool runs one only through its interpreter (`node scripts/x.js`).
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
  A directory any of these flags lists that lies inside the project, by its text (relative or absolute) or through a link to the project,
  is refused, exit 2, when it leads out of the project through a symlink, like `--allow-read=src` with a committed `src -> ~`,
  naming the real path to type instead, so a flag typed as documented never grants what a link in the tree points at.
  One through a symlink that stays inside is noted, as `fgro run` notes every one ([run](run.md#usage)).
  One that lands in a dot-entry or `node_modules` the path doesn't name, like `src/fixtures -> ../.git`, is refused, exit 2, naming that entry's path to type instead,
  as is a path typed outside the project through a link of your own into such an entry,
  so an entry the box leaves out is written only when a flag names it.
  `--allow-run-write=<program>:<dirs>` reads and writes directories for one program the command starts,
  named as the tool is called, like `vitest`, or as `--allow-run=` names it, or by its name in the run, like `node_modules/.bin/vitest`,
  each opened as a write flag's is, a rule of its own, and the root the same per-entry set.
  Gro's own code gains nothing there.
  A name the command may start no program by is refused, exit 2, naming those it may start,
  and so is a plain name both the project's own tool and an `--allow-run=` grant have, the project's own then named by `node_modules/.bin/<name>`.
  Where gro's own code writes the whole project, as under `gen`, `format`, or `sync`, or with `--allow-write=.`,
  a program's directory under a dot-entry or `node_modules` is refused, exit 2, unless `--allow-write=` names it too,
  since the box alone keeps gro's code out of those.
  So is one holding the project root, and for a command that may start a program one holding the OS temporary directory.
  Only the program it names writes there: `--allow-read=` and `--allow-run-in=` widen every program's box, `--allow-write=` none of them,
  so a tool's own write, like a test updating its snapshots or `eslint --fix`, takes `--allow-run-write=`.
- **A `node_modules` that's a symlink**, as agent worktrees and scratch copies make, is followed only to another checkout's:
  a directory named `node_modules` outside the project, neither inside it nor holding it.
  It's read there on its real path (and executed, for a command that may start a program), and one `note:` line names where it leads.
  Gro's own code reads through it too, as a read-only root of the run's `fs` on the same resolution the box follows,
  so a read the box allows there isn't refused as a symlink leaving the project.
  A package there resolves its imports within that `node_modules`, and the resolver reads nothing of the checkout holding it, its `package.json` included ([resolver](resolver.md#departures-from-node)).
  No tool directory is made there, since that checkout's own unconfined commands run what a tool writes there.
  A tool writes there only what `--allow-run-write=` names for it, by the path it leads to; `--allow-write=` naming it lets only Gro's own code write there;
  both write plain (no symlink, FIFO, or moved-in entry).
  A tool that needs a directory there fails without the flag, and the `note:` line names the tools that write there, if any,
  then, for the rest of the command's tools, says that the checkout's own commands run what they write and gives the whole command again with the flags that let each write its directories there, each quoted for a shell,
  after the command's own grant flags and before its args, so a paste can't put one where the command reads it as an arg.
  Under `fgro check` in a worktree linking a SvelteKit project's `node_modules` at `<target>`, that is
  `fgro check '--allow-run-write=svelte-check:<target>/.vite-temp' '--allow-run-write=svelte-kit:<target>/$app,<target>/.vite-temp' '--allow-run-write=vitest:<target>/.vite-temp,<target>/.vite/vitest'`,
  naming only the directories the project's dependencies call for, and `<target>` itself for a tool one of whose directories isn't a directory there yet (a symlink counts as none), which lets that tool write all of that `node_modules`.
  A `<target>` with no `package.json` beside it, so no checkout's, like a global npm prefix's `lib/node_modules`,
  and one a shell or the flag would read otherwise (a quote, a backslash, a comma, anything but printable ASCII) get the flag's shape alone.
  A link to any other directory, like `~/.ssh` or `.git`, is refused, exit 2.
- **Network:** the run's own namespace, whose only interface is its loopback,
  or where user namespaces are restricted the host's network with every internet socket refused ([namespaces](#namespaces)).
  So a test that connects to a service outside the run, like a database on the host, fails, its connection refused or its network unreachable.
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
  The run marks itself not dumpable before its box goes up, so no program in the box reads the run's own environment or memory either,
  and attaching to the run (`perf -p`, `gdb -p`) is refused, while starting `fgro` under `perf` or `gdb` works.
  The programs it starts are dumpable, but each runs in a box of its own, so no tool reads another's environment, memory, or working directory,
  while each still sees the others' command line and status.
- **Resource limits:** the `core` hard limit, the only one lowered, set to 0 before the box,
  which every program the run starts inherits, so a crash writes no memory or environment to disk,
  a guardrail against that accident and not a wall, since deliberate code writes what it holds into a write root;
  a process changes only its own limits, the denylist refusing a limit set on another.
- **The environment:** the run's programs see the run's own, as on Node, but for the temp and XDG variables above
  and the vendored `deno_core`'s diagnostic variables, `DENO_STARTUP_PHASES`, `DENO_LOG_LAZY_LOAD`, and `DENO_SNAPSHOT_IMPORT_GRAPH`,
  which the run removes for itself and its tools ([run](run.md#usage)).

The realm and the box are derived from the same run, and the box's plan feeds the realm one root back, the linked `node_modules` above.
The realm's roots are directories, and its write on the project is the project root;
the box is narrower on the project and wider for the tools.
So a write the realm admits and the box refuses, like a genfile's new top-level output or a write under `.git`, `node_modules`, or another dot-entry, fails as an `FsError` of kind `permission_denied`,
and a tool's own write fails with the kernel's `EACCES`, where the Node Gro writes each.
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
These refusals print no cause: a write through a dangling symlink, resolved only to its own parent,
and a recursive `mkdir` or copy through a symlink on the way.
When a run exits nonzero and a tool it started ended other than by exiting 0 (not by the run's own signal), the run ends with a `note:` line naming the box's network and writes, as in

```
note: fgro test's tools each ran in a kernel box of their own inside the run's, which lets them reach no network but the run's own loopback and write only to a temporary directory of their own and to their own directories (svelte-kit: .svelte-kit, node_modules/$app, node_modules/.vite-temp; vitest: .svelte-kit/generated/dev, .svelte-kit/types, node_modules/.vite-temp, node_modules/.vite/vitest, src): a tool that hits a wall fails with EACCES, EXDEV, EPERM, or ENOSYS; --allow-run-write=<program>:<dirs> lets one of them (svelte-kit or vitest) write more, --allow-read= and --allow-run-in= widen every box, and --allow-write= widens only what gro's own code writes
```

Both are built from the rules the box installed.
The cause names tool and flag directories: it leaves out the directories `--allow-run-write=` gives a program, as gro's own code can't write there,
and names the top-level entries only where gro's code writes the project.
The note, here for `fgro test --allow-run-write=vitest:src -- -u`, names each tool's own directories (the tool directories the box made for it and what `--allow-run-write=` gives it),
and the programs the command may start; it never names what `--allow-write=` names.
In the host's network the note's clause reads "reach no network".
Both are review signals, which the run's code can print as well.

### How it goes up

The binary takes the box's steps on its one thread, before V8 or any other thread starts, in an order that keeps a fallback an unchanged process:

1. The pre-pass reads the command's closure, the grant flags, and the project, and holds no descriptor when it returns.
   Every fallback the command line or the project's files decide runs the Node Gro here, so Node gets the process the Node Gro would have,
   and what it refuses, like `-A` before the command word, or `--allow-run-write=` naming a program the command may start by no name, exits 2.
   `-A` before the command word is refused as `-A comes before the command, and the binary runs each command of Gro's in a kernel box, which -A can't lift: …`,
   ending with the whole command on the Node Gro;
   before `run`, as in `fgro -A run x.ts`, the refusal says grant flags go after the command word, like `fgro run -A`.
2. It refuses what can't run boxed, before anything changes:
   a binary built without its snapshot, which would read its own sources from its build tree, exit 1,
   and `-A` after the command word, whose unconfined spelling is the Node Gro, exit 2, as
   `error: -A would run a command of Gro's unconfined, and the binary runs each in a kernel box: the project's Node gro is the unconfined spelling (node_modules/.bin/gro format --check)`.
3. It probes the kernel and the standard streams, changing nothing.
   Where the kernel's Landlock is missing, off, or below its sixth ABI, or a seccomp filter the process inherited refuses its calls, there's no box, so the command runs on the Node Gro after a `note:` line saying so, a fallback the host decides.
   `GRO_FALLBACK=command` refuses that fallback, exit 2 ([authority](authority.md#what-the-first-release-is-for)), and so does a project with no Node Gro installed.
   A socket on stdin, stdout, or stderr that isn't a connected unix stream is refused, exit 2, since a box can't hold where it leads,
   so an inetd-style launch fails, while a pipe, a file, a terminal, `/dev/null`, and a connected socket pair, like Node's `spawn` makes, pass.
4. Then the first change: it closes every descriptor it inherited above stderr, and from here nothing falls back.
   It enters the network namespace, then drops its capabilities, marks itself not dumpable, lowers the `core` limit, and raises its open-files soft limit, as Node does.
5. It opens the OS's temporary directory by its real path and removes the stale private directories there, unless it lies in the project.
6. It resolves once what the run and its box both use: the directories the grant flags list, the programs it pins, `node` among them, and the working directory,
   refusing, exit 2, a directory listed inside the project that leads out of it through a symlink, or through one into a dot-entry or `node_modules` it doesn't name.
7. It derives the box's plan from that resolution and the disk's facts, read once, and prints the plan's note.
   The plan refuses, exit 2, a write flag or a program's write directory naming a directory that holds the project root;
   a program's write directory under a dot-entry or `node_modules` where gro's own code writes the whole project and no `--allow-write=` names it, naming the command;
   a `node` on `PATH` that's a script, like an asdf or nodenv shim, which reads its manager's tree under the home directory, naming the shim and the fix
   (the real `node` first on `PATH`, or `--allow-run-in=` naming the version manager's directory), unless an `--allow-run-in=` names the directory it's in,
   while a native shim, like volta's, can't be told apart from `node`, and its tools fail inside the box;
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

So a command of Gro's on the binary exits with Gro's code as a run does, and `2` when it's refused before anything runs:

- its grant flags, `-A` before or after the command word among them
- a write flag or a program's write directory naming a directory that holds the project root
- a directory a grant flag lists inside the project that leads out of it through a symlink, or through one into a dot-entry or `node_modules` it doesn't name
- `--allow-run-write=` naming a program the command may start by no name, or by one two programs share
- a program's write directory under a dot-entry or `node_modules` where gro's own code writes the whole project and no `--allow-write=` names it too
- `GRO_FALLBACK`'s value, a working directory with no project's `package.json`, or a project's `package.json` or installed Gro's that can't be read
- a forwarded section turning off a check the command's invoke runs, like `-- gro gen --no-check` after `check` ([task](task.md#a-builtins-host-and-grants))
- a `node` on `PATH` that's a script
- a `node_modules` symlink to anything but another checkout's `node_modules`, or git's files the box can't read for a command that may start a program
- a socket on stdin, stdout, or stderr that isn't a connected unix stream, or an OS temporary directory inside the project
- a flag or rule a program's box takes holding the OS temporary directory for a command that may start a program (`--allow-read=/tmp`, `--allow-read=/`, `--allow-run-write=vitest:/tmp`, never `--allow-write=`)
- a path planted where the box opens a rule or makes a tool directory, like a symlink
- a fallback `GRO_FALLBACK=command` refuses, the one the host decides where Landlock can't hold the box among them, or a fallback with no installed or built Node Gro or no `node` to run it

It exits `1` when the grants table or the working directory can't be read, `node` fails to start, the binary was built without its snapshot,
or a step of the box fails once the first has changed the process (a namespace left unusable, a tool directory that can't be made, where a linked `node_modules` leads that can't be looked at, a process with more than one thread),
and with the Node Gro's code when it falls back, where Landlock can't hold the box too.

### When a command hits a wall

A tool that meets the box fails with the kernel's own error:
`EACCES` for a path its rules leave out,
`EXDEV` for a move or hard link between two directories where either is or lies in a directory the box writes plain and lies in none it writes whole (a tool directory, or a directory `--allow-run-write=` names that is, lies in, or holds one or a linked `node_modules`'s target),
or into one that grants more than where the entry is (a tool's temporary directory, which is executable, from the project),
`EACCES` taking precedence where the box also refuses writing at either end of a move, or making the entry where it lands, like a symlink in a plain directory,
`EPERM` for a signal to a process outside the run (like an editor, or a dev server it didn't start) or a change of a file's owner,
and `ENOSYS` for a call family the box refuses.
Gro's own code fails with a `permission_denied` `FsError`, whose cause names where the box lets the command write when the path lies outside it, and a `rename` the box refuses between two directories with `io_error`, its cause the kernel's words ([fs on fgro](host_modules.md#fs-on-fgro));
and a run that fails after a tool did ends with a `note:` line naming the box's network and writes ([above](#what-a-commands-box-admits)).
The grant flags, right after the command word, widen the box for one run:

- a read outside the project, like a tool's config or cache under the home directory, which the tool otherwise finds unreadable, failing with `EACCES` or running as though there's none, takes `--allow-read=<dir>`
- a write of Gro's own code outside the write roots takes `--allow-write=<dir>`, naming the directory itself, never one that holds the project root
- a tool's own write, like a test updating its snapshots or `eslint --fix`, takes `--allow-run-write=<program>:<dirs>`, like `gro test --allow-run-write=vitest:src -- -u`
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
(`.svelte-kit/generated/dev` holds the modules a dev server loads),
and under `gen`, `format`, or `sync` any included entry the next unconfined command runs, like `src`, `package.json`, or `vite.config.ts`,
which is why a check-mode command writes none of them.
An exec root that is also a write root accepts the same case for the run's own programs, so each is declared and reviewed as narrowly.
Each tool runs in a box of its own and writes only its own tool directories, but a tool directory two tools write is each one's,
so one can plant there what the other reads: svelte-check's and Vitest's `.svelte-kit/generated/dev`,
and the `node_modules/.vite-temp` of `svelte-kit`, svelte-check, and Vitest; and `svelte-kit` writes all of `.svelte-kit`, Vitest's own directories in it among them.

### A box for each tool

Each program the run starts runs in a box of its own, nested in the run's (`gro_runtime`'s `confine/program.rs`),
built at the program's first launch and kept for every launch after, one box for each name the run grants a program by and one for each program found beneath an exec root,
and installed at every launch of a command of Gro's; `fgro run` launches in none.
A box that can't be built fails each launch of its program, `launch_failed`, as the first did.
Landlock restricts the thread that installs its rules, so a launch given a program's box starts the program from a short-lived thread that first restricts itself with that box
and then spawns it as today, and the program inherits the narrower box (`host/spawn/launch.rs`); nothing re-executes the binary.
Each program's box gives it a temporary directory of its own, named by the program's place among the run's programs, never by its name.
A program of the grants table writes the tool directories its grant's `write` names, of those the run made, with no symlink, FIFO, or moved-in entry.
Any other program, like one a flag grants, writes only its own temporary directory, and so does one of the table whose grant names none: tsc, ESLint, git, and npm
([above](#what-a-commands-box-admits) has the tools that name some).
Each program also writes what `--allow-run-write=` names for it, and nothing another program's flag names.
No program's box takes anything `--allow-write=` names, not even its read; a tool that must read what Gro's code wrote outside the project takes `--allow-read=`.
The nested boxes also keep a tool from signalling Gro or another tool and from reading another's environment, memory, or working directory through `/proc`.
So a test or an ESLint plugin can't plant in what `vite build` runs, `.svelte-kit/generated/build`,
in the optimized dependencies a later `dev` loads from `node_modules/.vite`, in the tsconfig and types `node_modules/$app` hands every later checker,
or in fuz_css's `.fuz/cache`, and no tool leaves a symlink in a tool directory or a linked `node_modules` for a later one to write through.
It leaves the project's own config, which runs in every tool and which `dev` runs unconfined,
and a write into a tool's own directory that a dev server already running picks up, like a test's into `.svelte-kit/generated/dev`.
A tool still reads what the others write in the project, sees their command line, status, and descriptor numbers in `/proc`,
gets a handle from `pidfd_open` on another or on the run, one that can't signal, and can take the terminal's foreground.
A child that sets `PR_SET_PDEATHSIG` while the short-lived thread that started it still runs is killed when that thread ends,
and a signal sent to that thread alone as it ends is lost.

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
