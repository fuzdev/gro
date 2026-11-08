# host modules

> the imports that reach the OS, `fs` and `spawn` so far: how to import them, hand them down,
> and what the Node implementation does and doesn't guarantee

> **status**: `fs` and `spawn` exist as the Node implementation in Gro's npm package,
> `spawn` with inherited and captured calls, not yet long-lived children;
> process, watch, and net are designed, not built.
> On `fgro` the host modules will be builtins resolved ahead of package lookup
> ([runtime](runtime.md#imports-or-globals) has the design).

Host modules are Gro's own modules under `@fuzdev/gro/*`.
Each is an import, so what a module can reach shows in its imports,
and narrowing it means handing down a narrower handle.

## fs

```ts
import {fs} from '@fuzdev/gro/fs.ts';

const src = fs.project.dir('src'); // a handle on `src/`, which must exist
const text = await src.read_text_if_exists('notes.md'); // `null` when missing
await fs.project.write('.gro/out.json', JSON.stringify({text}));
```

- `fs.project` is the project directory, an `FsDir`.
  `fs.open_dir(absolute_path)` opens another,
  and `fs.temp_dir()` makes a fresh one removed on `await using`.
- Paths beneath a handle are relative and `/`-separated, resolved as `openat2` with `RESOLVE_BENEATH` does.
  An absolute path or `..` above the handle fails with `outside_root`,
  and so does an absolute symlink, or a relative one whose target climbs above the handle,
  even when either would land back inside.
- A path's own `..` segments resolve lexically, before symlinks, so `link/..` is the handle's directory.
  The runtime's native op normalizes them the same way before calling the kernel, a requirement on its wrapper,
  since the kernel would resolve them physically.
- A trailing `/` means a directory, as in the kernel: `stat('a.txt/')` is `not_a_directory`,
  `write('new.txt/')` fails, and `remove('link/')` and `rename('link/', …)` refuse a symlink
  with `not_a_directory` rather than act on what it points at.
  A symlink whose target passes through a file (`a.txt/`, `a.txt/..`) is `not_a_directory` too.
- `remove` and `rename` refuse the handle's own directory (`''`, `'.'`) with `io_error`;
  its parent's handle removes it.
- `dir.dir(path)` narrows to a subdirectory and `dir.readonly()` drops the write ops,
  at runtime as well as in the types: handles come only from `fs` and other handles,
  and they and their prototypes are frozen, so no holder reaches past its handle through `constructor`
  or patches the ops another holder calls.
  Every op reads its handle's state from private fields, never `path`,
  so an op called on any other object (`proto.remove.call({path: '/'}, …)`) throws `TypeError`.
- `dir.path` is the absolute real path, for display and a spawned process's cwd.
  It carries no authority.
- Errors are `FsError`, typed by `kind` (`not_found`, `already_exists`, `permission_denied`,
  `not_a_directory`, `is_a_directory`, `not_empty`, `outside_root`, `io_error`).
  Match one with `is_fs_error(error, kind?)`, which also recognizes one from another copy of Gro.
  Both live in `@fuzdev/gro/fs_error.ts`, apart from the host module,
  so code that only makes or matches an error imports no capability.
  It's part of the host-module surface: on `fgro` it's a builtin that grants nothing,
  resolved beside `fs`, so the builtin and every importer share one `FsError` class.
  An error's `path` is the absolute path the op was asked for, or for `outside_root` the path as given,
  never a path a symlink led to.
- Each op's one expected failure returns a value instead of throwing:
  `stat` returns `null`, `exists` returns `false`, `read_if_exists` and `read_text_if_exists`
  return `null`, `remove({missing_ok})` and `mkdir({exists_ok})` return,
  and `write({create_new: true})` returns `'exists'`.
  Any other failure still throws, so a permission error is never read as a missing file,
  and a path through a file (`f.txt/x`) is `not_a_directory`, not missing, sync and async alike.
- `stat_sync`, `exists_sync`, and `read_text_sync` are the sync ops, for hot build paths.

The module's TSDoc has each op's contract.

`fs_search` (`@fuzdev/gro/fs_search.ts`) is library code over a handle,
finding the files beneath a directory through `list`, recursively.
A symlink is a result like a file, never followed, its `kind` telling them apart,
and a missing directory finds nothing.
Gro's own sweeps, like `gro format`'s, act on regular files only and skip a symlink, as tsv's walk does.

## spawn

```ts
import {spawn} from '@fuzdev/gro/spawn.ts';

const git = spawn.program('git', {pass_env: ['GIT_*', 'SSH_AUTH_SOCK']});
const status = await git.out(['status', '--porcelain']);
if (status.kind === 'exited' && status.code === 0) console.log(status.stdout);
await git.run(['fetch'], {cwd: repo_dir}); // `repo_dir` an fs handle, stdio inherited
```

- `spawn.program(name, options?)` makes a `SpawnProgram`, the capability to run one executable,
  resolved once when it's made and pinned for the handle's life.
  `spawn.find(name, options?)` is the same, returning `null` instead of throwing `not_found`,
  and any other failure, like `not_executable`, still throws.
- A bare name walks the host's `PATH` synchronously, absolute entries only, to the first regular executable.
  Neither `program` nor `find` looks in `node_modules/.bin`, so a dependency's `.bin/git`
  can't shadow `spawn.program('git')`.
- A name containing `/` resolves against the project root. It's a lexical check:
  a name that's absolute or climbs above the project through `..` is `not_granted`,
  while a symlink beneath the project is pinned wherever it leads, as a `PATH` entry would be.
- The pin is the executable's real path. A later `PATH` change or swapped symlink changes nothing.
  The real path is what runs. A bare name is its own `argv[0]`,
  since a multicall binary like `~/.cargo/bin/cargo -> rustup` picks its behavior from `argv[0]`,
  and a name containing `/` passes its lexical project-absolute path,
  so a tool finding itself through `argv[0]`, like a virtualenv's `python`, still does under another `cwd`.
  `name` and `path` are for display; the pin itself is private.
- A file is pinned only when its first bytes are a format the kernel runs, else it's `not_executable`,
  an empty file included, since on Linux `execvp` hands any other file the kernel won't run to `/bin/sh`.
  `#!` is accepted everywhere, the ELF magic everywhere but macOS,
  and on macOS the Mach-O magics, 32-bit and 64-bit in either byte order and fat binaries.
  A file with execute permission this user can't read passes, as it can't be inspected.
  `binfmt_misc` formats are refused along with the rest.
- `program.run(args, options?)` runs with stdio inherited.
  `program.out(args, options?)` gives the child `/dev/null` for stdin and captures stdout and stderr as UTF-8 text,
  with `U+FFFD` for bytes that aren't, up to `max_bytes` per stream, 64 MiB by default.
  Output past the cap kills the child with `SIGKILL` and the result has `truncated: true`,
  its `kind` then `signaled` unless the child had already exited,
  and a multi-byte character cut at the end becomes `U+FFFD`.
- A result is `{kind: 'exited', code}` or `{kind: 'signaled', signal}`, so a non-zero exit is a value.
  A program that can't start throws `SpawnError` of kind `launch_failed`.
  `not_found`, `not_executable`, `not_granted`, and `launch_failed` are the kinds,
  matched with `is_spawn_error(error, kind?)`. Both live in `@fuzdev/gro/spawn_error.ts`, apart from the host module,
  so matching one grants nothing; on `fgro` it's a builtin that grants nothing, resolved beside `spawn`.
- `.bat` and `.cmd` are refused, by the name and by the pin's real path. A call is argv only, never a shell string.
- A name or argument must be a string without a NUL byte or a lone surrogate, else a `TypeError`,
  and a name can't be empty, while an argument can (`git commit -m ''`).
  A lone surrogate would reach a child as U+FFFD, a different string than the caller wrote.
- `cwd` is an fs handle, never a string, and defaults to the project root.
- A child starts from a cleared environment, on the runtime's grant rules:
  `PATH`, `HOME`, `TMPDIR`, and `TERM` when the host has them, `LC_ALL=C`, and the host's own
  `NO_COLOR` and `FORCE_COLOR`, then the names `pass_env` lists, then the values `env` gives.
  `pass_env` takes names and `PREFIX_*` globs; `*` alone is refused, since `'all'` says that out loud.
  Environment options exist only on `program`, never per call,
  because a per-call `LD_PRELOAD` or `GIT_SSH_COMMAND` would turn a `git` handle into arbitrary execution.
  The set is fixed when the handle is made.
- `pass_env: 'all'` is the hatch, as strong as `-A`: the child gets the host's whole environment
  and no base pinning (`LC_ALL` included), for forwarding tasks where a person made that choice.
- `spawn.narrow(names)` is a root limited to `names`, spelled as they'll be requested,
  and it only shrinks: narrowing to a name its parent doesn't allow is `not_granted`.
  A name outside the set is `not_granted` from `program` and `find` alike.
  A narrowed root also refuses `env`, `pass_env: 'all'`, and `pass_env` globs as `not_granted`, through any chain of narrows.
  `env` would turn a named program into arbitrary execution,
  and `'all'` or a set of globs would let the holder of a narrowed handle take the whole environment.
  Named `pass_env` stays, forwarding one host value at a time, so a narrowed caller names each variable.
  To give a library a program with an environment, build the `SpawnProgram` at the root and hand that down.

The module's TSDoc has each op's contract.

## Where to import them

- Import host modules at the [composition root](glossary.md#composition-root),
  the task or entry module where authority enters,
  so the policy diff points at a few files a person can review.
- Library code takes capabilities as parameters, through small dependency interfaces,
  the way a function takes its dependencies.
  Tests then pass real handles on a temp directory or plain-object mocks, with no module mocking,
  and a library given fs needs no policy entry of its own, since handing is delegation, not a new grant.
- Name a handle's type with `import type`, never `import {type FsDir}`.
  Under `verbatimModuleSyntax` the second form keeps the import,
  so the host module still links and the library looks like it reaches fs.
- Hand down the narrowest handle (`dir.dir('sub')`, `dir.readonly()`) rather than the module,
  so narrowing happens at the root, where review sees it.
- Gro's own code imports its host modules relatively (`./fs.ts`), like the rest of its `src/lib`;
  `@fuzdev/gro/fs.ts` is the specifier for every other package.
- A path that arrives from outside, like a CLI arg, a config value, or one of Gro's well-known paths,
  becomes a handle at one boundary, an `FsPaths` (`@fuzdev/gro/fs_paths.ts`).
  A composition root builds one from the root, `create_fs_paths(fs)`, and passes it down;
  it opens any path, so nothing builds one from a handle.
  It resolves a relative path against `fs.project`, follows the path's symlinks the way `fs.open_dir` does,
  the same inside and outside the project, then hands back `fs.project` for a real path beneath the project,
  or else a handle on its parent. Its `remove_path` and `empty_dir_path` refuse the protected directories
  [below](#gros-guards-on-its-own-destructive-paths).
  Gro's own code reaches every path it names through it, its well-known paths like `build/` and `.gro/` too,
  and resolves beneath a handle only what it got from that handle, like a listing's entries,
  so confinement applies to those.
  Every library module that reaches the filesystem takes an `FsPaths` or a narrow handle as a parameter,
  and only roots import `fs` and build one: the builtin tasks, the CLI's entry (`invoke.ts`),
  genfiles, the default config, and the plugin modules like `gro_plugin_sveltekit_library`.
  The CLI's entry passes its own to the config's loading and `invoke_task`,
  which carries it in its options to the tasks a task invokes, never on the task's context.
  The rendered `$app/env/public` and `$app/env/private` modules are roots too,
  importing `fs` and `create_fs_paths` to read the `.env` files when they run,
  so a module importing `$app/env/*` shows that read in its import graph,
  while the loader that renders them never loads the host module on its hooks thread.
  Gro reads its own `package.json` through the module system (`package_json_load_for_gro`), with no fs grant.
  A function that reads files outside the directory it's named for takes an `FsPaths` rather than a handle,
  like `library_load_from_repo(fs_paths, repo_dir)`,
  since a repo's `package.json`, `tsconfig.json`, or `.gro/` can be a symlink out of it,
  which a handle on the repo would refuse.
  Where a directory's files come from its own listing or search, Gro opens the directory once
  and reads and writes them beneath its handle.
- These library modules reach further than the parameters they're given, until the ops they need exist.
  `cli.ts` imports `fs` itself for `find_cli`'s check of a local `node_modules/.bin`,
  since its callers, through `spawn_cli`, are library code too;
  it takes an `FsPaths` with the spawn host module, from the roots that find CLIs.
  And the boundary's machinery (`internal/fs_boundary.ts`) reads real paths and symlinks from `node:fs` itself,
  so an `FsPaths` reaches the filesystem beyond the `Fs` it's built from until that read is an op on the root,
  beside `open_dir`, reached through the `Fs` passed in.
- Gro's library and task code otherwise reaches the filesystem only through the fs host module,
  so it can run unchanged where the module is a builtin.
  The rest that stays on `node:fs` is Node-only machinery: the loader and the modules it runs in its hooks
  (`svelte_config.ts`, `sveltekit_shim_app_env_vars.ts`), the Filer family and the Node module graph,
  the check of the working directory that `fs.project` and `paths` load from (`internal/cwd.ts`),
  the bootstrapping that finds the Gro install to spawn `node` with (`gro_helpers.ts`),
  and the sync stats behind the task-path hints (`internal/task_hints.ts`),
  which the synchronous args parsing calls, where no sync boundary opens a path from outside.

## Hardening

### The module's invariants

The fs module keeps these in the Node implementation, and the runtime's builtin keeps them natively:

- Handles come only from `fs` and other handles.
  Each constructor checks a key only the module holds, so a handle's `constructor` makes nothing.
- A handle's state lives in private fields, and every op reads it from there,
  so an op called on a borrowed receiver, any object but a handle, throws `TypeError`.
- Handles, their prototypes, `fs`, and `FsError` with its prototype are frozen.
- `remove` and `rename` refuse the handle's own directory.
- A path's `..` segments resolve lexically, before any symlink, and never climb above the handle.
  A path is a string without a NUL byte or a lone surrogate, which would encode as U+FFFD
  and name a different entry than it spells, else a `TypeError`.
- An absolute symlink is `outside_root`, and so is a relative one that climbs above the handle,
  even when it would land back inside. A component a path passes through must be a directory.
- `fs.temp_dir` takes only a string prefix, so no `toString` of the caller's names the directory.

### spawn's invariants

- Handles come only from `spawn` and other handles, through a module-private key,
  and hold their pin, `argv[0]`, and environment in private fields,
  so a method called on any other object throws `TypeError` and changing `path` changes nothing that runs.
- Handles, their prototypes, `spawn`, results, and `SpawnError` with its prototype are frozen.
- A result is a null-prototype record, so a polluted `Object.prototype.then` isn't consulted when it's awaited.
- A child's environment is a null-prototype object, since Node enumerates it with `for in`
  and `Object.prototype.X` would otherwise reach the child.
  The options to Node are null-prototype too, on top of Node's own null-prototype copy of them.
  `process.env` is read through own properties only, so a polluted or inherited name is no variable.
- Each option's value is read once and checked, before the name resolves, then only the checked copy is used.
  Only own properties count as options, so a polluted `Object.prototype.pass_env` sets nothing on a plain `{}`.
- Every array argument (`args`, `narrow`'s names, `pass_env`) is read by index with its length read once.

### The Node implementation's own measures

These exist because the Node implementation resolves paths itself, in JS, before handing one to Node:

- The walk takes one component at a time, and a component holds no `/`,
  checked by index, so a segment a polluted `split`, `pop`, or array iterator forged is `outside_root`.
- Inside an op, errors are classified by a private brand, never `instanceof`,
  so no `Symbol.hasInstance` runs mid-op.
- The Node functions the module calls are copied at load,
  so `syncBuiltinESMExports` can't rebind them under a handle.
- The walk is synchronous, in the async ops too, reading components `realpath` just resolved,
  so a path through a symlink doesn't pay a threadpool trip per component.
  It blocks the event loop for those lookups, bounded by the path's components and `MAX_SYMLINKS`,
  and on a network or FUSE filesystem for however long the server takes to revalidate them.
- Symlink targets, real paths, and listed names are read as bytes and must be UTF-8,
  else the op fails with `io_error`, rather than a lossy name reaching a different entry
  or the kernel following bytes the walk never saw.
  One name that isn't UTF-8 fails the whole listing, so a sweep over the directory,
  like `gro format`'s, fails loudly rather than skipping or misnaming the entry.

### Gro's guards on its own destructive paths

Gro's tooling, not the module, guards what it removes and writes:

- Emptying or removing a path from outside refuses `/`, the project, `$HOME`, and an ancestor of either,
  and an `FsPaths` over another project still refuses the working directory Gro loaded in and its ancestors.
- `gro clean` empties a symlinked `.gro/`'s target only when it holds the marker Gro writes
  into a `.gro/` it creates, and isn't inside a git work tree.
  It leaves a dangling link alone, with nothing to clean.
  It unlinks `.gro/` with a warning when the target is a directory outside the project
  that lacks the marker or is inside a git work tree,
  and unlinks it silently when it points inside the project, at a file, or into a loop.
- Gro reads its working directory's real path as bytes when it loads,
  and refuses to load where that path isn't UTF-8, which `process.cwd()` would report
  lossily as a different directory.
- `gro deploy` refuses a deploy dir it didn't clone, `gro gen` writes only inside the project,
  and `gro upgrade` deletes only inside it.

## The Node implementation's gaps

The types are the runtime's, and the Node implementation runs the same ops,
but Node can't enforce what the runtime will.
It documents each gap rather than weakening the API to match, and is a review signal, not a boundary:

- Confinement is a check, then an open: a path is walked beneath the handle's directory,
  reading each symlink, then the op runs. A symlink swapped in between escapes, since Node has no `openat2`.
  Recursive `copy` and `remove` hand the whole tree to Node, which walks it by path.
- A handle is bound to its directory's path, not its inode.
  When the directory is moved or replaced, the handle follows the path,
  refused only when the path now resolves elsewhere (`outside_root`) or nowhere (`not_found`);
  the runtime's handle holds a descriptor and keeps the directory it opened.
- Intrinsics stay pollutable. Code that patches `String.prototype`, `Array.prototype`, `Promise`,
  `Error`, `TextDecoder`, or `Reflect` after the module loads runs inside its ops,
  and the module blocks only the forgeries named above.
- A handle opened on `/proc` follows its magic links (`/proc/self/fd/1`, `/proc/self/root`),
  which the kernel resolves without a path the walk can check.
- `fs.open_dir` takes any absolute path, checked against no grant.
- Any module can import `node:fs` and the `process` global stays,
  so a narrowed handle bounds only the code that uses handles.
- `fs.project` is the working directory when the module loads, as for Gro's `paths.root`,
  so importing the module throws when the working directory has been removed.

### spawn

- The pin is a path, resolved and then exec'd by path when called,
  so a file replaced at the pinned path in between runs instead.
- A forged `cwd` handle is any object with a checked absolute `path`, so a child can start in a directory it was never given.
  Only the shape is checked.
- A real `cwd` handle follows its path too. Where `fs` refuses a directory that was replaced
  (`open_dir(mv)`, then a rename and a symlink at `mv`, so `mv.exists('.')` is `outside_root`),
  a call with that `cwd` runs in what the symlink leads to.
- No policy check limits which programs a handle reaches or what they do,
  and no kernel confinement like Landlock applies to a child,
  so a running program reaches whatever its user can.
- A name's `/` confinement checks the name's spelling, so a symlink under the project reaches outside it.
- The refusal of a file with no `#!` reads only its first bytes, so a file with a native magic the kernel won't load
  or a `#!` line naming no interpreter, still reaches the Linux `/bin/sh` fallback of the `execvp` Node starts children with, and a Java class file passes on macOS, sharing the fat binary's magic.
- A shebang script's `$0` is the pinned path, not the name it was granted,
  so a shell script dispatching on the symlink it was called through can't.
- A child killed by a real-time signal is reported as `{kind: 'exited', code: 0}`, since Node passes an empty signal name for one.
- Intrinsics stay pollutable. The module copies at load the Node functions it calls,
  `Array.isArray`, `Number.isSafeInteger`, and `Object`'s `assign`, `create`, `freeze`, `keys`, and `hasOwn`,
  and its own objects are null-prototype, but code that patches `Array.prototype`, `String.prototype`,
  `Promise`, `Uint8Array`, `TextDecoder`, or `Error` after the module loads runs inside its ops.
- Node copies a set `NODE_V8_COVERAGE` into every child's environment.
- `spawn`, like `fs`, reads the project root when the module loads, so importing it throws
  in a removed or non-UTF-8 working directory.
- Any module can import `node:child_process`, so a narrowed `spawn` bounds only the code that uses it.

## Native requirements

What the runtime's builtin must do that the Node implementation can't:

### fs

- Make each confinement decision in one op, `openat2` with `RESOLVE_BENEATH` beneath the handle's descriptor,
  never a check followed by an open.
- Pass `RESOLVE_NO_MAGICLINKS` explicitly, so a handle on `/proc` refuses its magic links.
- Copy and remove a tree recursively beneath the descriptor, each entry opened relative to its parent's.
- Check `write`'s data is a string or a `Uint8Array`, and mask its `mode` to `0o777`.
- Return owned buffers from `read`, never a view of memory the runtime reuses.

### spawn

- Start a child from one op taking a program resource, argv, and a cwd resource,
  so a call names nothing a path could swap: the pinned program runs by descriptor where the kernel allows it,
  the cwd by its directory handle. A shebang script runs by path, since `/proc/self/fd` can't reach one.
- Clear the environment in the op (`env_clear`) and set only what the handle holds,
  with no way for a call to add to it.
- Inherit only fds 0, 1, and 2 into the child, and confine it with the run's Landlock rules.
- Refuse a name or argument holding a NUL byte or a lone surrogate before it reaches the kernel.
- Report a child killed by any signal, real-time ones included, from `wait4`'s status.
- Resolve the `PATH` walk and the realpath pin at grant time, so a handle's reach is decided once.
