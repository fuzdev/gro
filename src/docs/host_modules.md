# host modules

> the imports that reach the OS, starting with `fs`: how to import them, hand them down,
> and what the Node implementation does and doesn't guarantee

> **status**: `fs` exists, as the Node implementation in Gro's npm package;
> process, spawn, watch, and net are designed, not built.
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
  becomes a handle at one boundary, Gro's internal `fs_open_path`.
  It follows the path's symlinks the way `fs.open_dir` does, the same inside and outside the project,
  then hands back `fs.project` for a real path beneath the project, or else a handle on its parent.
  Gro's own code reaches every path it names through it, its well-known paths like `build/` and `.gro/` too,
  and resolves beneath a handle only what it got from that handle, like a listing's entries,
  so confinement applies to those.
  The target is for the tasks, the composition roots, to thread handles down to the library code they call,
  which isn't done yet, so some of Gro's library modules still open paths through `fs_open_path` themselves.
  Where a directory's files come from its own listing or search, Gro opens the directory once
  and reads and writes them beneath its handle.
- Gro's library and task code reaches the filesystem only through the fs host module,
  so it can run unchanged where the module is a builtin.
  What stays on `node:fs` is Node-only machinery: the loader and the modules it runs in its hooks
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

### The Node implementation's own measures

These exist because the Node implementation resolves paths itself, in JS, before handing one to Node:

- The walk takes one component at a time, and a component holds no `/`,
  checked by index, so a segment a polluted `split`, `pop`, or array iterator forged is `outside_root`.
- Inside an op, errors are classified by a private brand, never `instanceof`,
  so no `Symbol.hasInstance` runs mid-op.
- The Node functions the module calls are copied at load,
  so `syncBuiltinESMExports` can't rebind them under a handle.
- Symlink targets, real paths, and listed names are read as bytes and must be UTF-8,
  else the op fails with `io_error`, rather than a lossy name reaching a different entry
  or the kernel following bytes the walk never saw.
  One name that isn't UTF-8 fails the whole listing, so a sweep over the directory,
  like `gro format`'s, fails loudly rather than skipping or misnaming the entry.

### Gro's guards on its own destructive paths

Gro's tooling, not the module, guards what it removes and writes:

- Emptying or removing a path from outside refuses `/`, the project, `$HOME`, and an ancestor of either.
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

## Native requirements

What the runtime's builtin must do that the Node implementation can't:

- Make each confinement decision in one op, `openat2` with `RESOLVE_BENEATH` beneath the handle's descriptor,
  never a check followed by an open.
- Pass `RESOLVE_NO_MAGICLINKS` explicitly, so a handle on `/proc` refuses its magic links.
- Copy and remove a tree recursively beneath the descriptor, each entry opened relative to its parent's.
- Check `write`'s data is a string or a `Uint8Array`, and mask its `mode` to `0o777`.
- Return owned buffers from `read`, never a view of memory the runtime reuses.
