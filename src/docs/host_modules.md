# host modules

> the imports that reach the OS, `fs`, `spawn`, `net`, `watch`, and the process modules (`env`, `args`, `exit`,
> `stdin`, `stdout`, `stderr`, `cwd`, `pid`, `runtime_version`), and `blake3` and `tsv`, which reach nothing:
> how to import them, hand them down, and what the Node implementation does and doesn't guarantee

> **status**: every host module here exists as the Node implementation in Gro's npm package.
> On `fgro` the host modules are designed as [builtin modules](glossary.md#builtin-module), resolved by the binary ahead of package lookup,
> and [the table below](#the-modules) marks the ones built
> (the others' specifiers are refused there as not on `fgro` yet, in any spelling, never loaded from the npm package; [on `fgro`](#the-process-modules-on-fgro));
> everything else on this page is the Node implementation's, but the sections "on `fgro`" and [native requirements](#native-requirements), which are the runtime's
> ([runtime](runtime.md) is the hub of the runtime's docs).

Host modules are Gro's own modules under `@fuzdev/gro/*`.
Each is an import, so what a module can reach shows in its imports,
and narrowing it means handing down a narrower handle.

## The modules

Every host module, each `@fuzdev/gro/<module>`, and whether it's a builtin module on `fgro` yet.
This table is the reference for what's on `fgro`.

| module               | what it reaches                         | on `fgro`                |
| -------------------- | --------------------------------------- | ------------------------ |
| `fs.ts`              | the filesystem, through handles         | built, the `fs` grant    |
| `spawn.ts`           | programs, as child processes            | built, the `spawn` grant |
| `net.ts`             | the network, by host                    | not yet                  |
| `watch.ts`           | changes under a directory handle        | not yet                  |
| `env.ts`             | the environment                         | built, the `env` grant   |
| `args.ts`            | the script's arguments                  | built, the `args` grant  |
| `exit.ts`            | how the run ends                        | built, the `exit` grant  |
| `stdin.ts`           | standard input                          | not yet                  |
| `stdout.ts`          | standard output                         | not yet                  |
| `stderr.ts`          | standard error                          | not yet                  |
| `cwd.ts`             | nothing: the working directory, a value | built, no grant          |
| `pid.ts`             | nothing: the process's id, a value      | built, no grant          |
| `runtime_version.ts` | nothing: the runtime's name, a value    | built, no grant          |
| `blake3.ts`          | nothing: BLAKE3 hashing over bytes      | built, no grant          |
| `tsv.ts`             | nothing: tsv's formatters and parses    | built, no grant          |
| `env_error.ts`       | nothing: `env`'s error class            | built, no grant          |
| `fs_error.ts`        | nothing: `fs`'s error class             | built, no grant          |
| `spawn_error.ts`     | nothing: `spawn`'s error class          | built, no grant          |
| `net_error.ts`       | nothing: `net`'s error class            | not yet                  |
| `stdio_error.ts`     | nothing: the streams' error class       | not yet                  |

A module that isn't on `fgro` yet is refused there by its specifier, in either spelling, never loaded from the npm package.

Each host module's errors are one class, exported by its error module with a guard, typed by `kind` and naming a target
(`FsError`'s `path`, `SpawnError`'s `program`, the others' `target`).
Its `message` is exactly `kind: target`, on both runtimes and every filesystem, and its constructor is `new XError(kind, target, options?)`, taking no message.
What the OS or the module said of a failure is its `cause`, by one rule:
where the module has an explanation of its own, the `cause` is a plain `Error` holding it,
like `../x climbs above /p` for an `outside_root` or `tool at /p/tool can't be read` for a `not_executable`,
whose own `cause` is the error the failure came from, when there is one (Node's error, an abort's reason, the `FsError` a spawn lookup met);
where the module has nothing to add, the `cause` is that error, unchanged (an fs op's Node error, a stream's error);
and an error with nothing to say beyond its kind and target has no `cause`, like `env`'s `not_granted`, a walk's `not_found` or `not_a_directory`, and `watch.dir` on a missing directory.
So a program that prints `message` prints the same text wherever it runs, and one that shows a failure to a person prints the chain of causes too,
as `format_error_detail` from `@fuzdev/gro/error_detail.ts` does: the message, then each cause's in parentheses, control characters escaped.
Gro prints the chain when a task fails, a task or genfile module fails to import, a genfile fails, a deploy's build fails, an error goes uncaught, a program can't be started, a path can't be read, and in its warnings, after the stack where it prints one.

## fs

```ts
import {fs} from '@fuzdev/gro/fs.ts';

const src = fs.project.dir('src'); // a handle on `src/`, which must exist
const text = await src.read_text_if_exists('notes.md'); // `null` when missing
const out = await fs.open_dir(`${fs.project.path}/out`); // on `fgro`, the directory `--allow-write=out` grants
await out.write('notes.json', JSON.stringify({text}));
```

- `fs.project` is the project directory, an `FsDir`
  (on `fgro`, the entry's [project root](glossary.md#project-root); here the working directory, [a gap](#the-node-implementations-gaps)).
  `fs.open_dir(absolute_path)` opens another,
  and `fs.temp_dir()` makes a fresh one removed on `await using`.
- `fs.real_path(absolute_path)` returns a path's real path, every symlink followed wherever it leads.
  It has `open_dir`'s reach, any absolute path, so it's on the root and not a handle:
  beneath a handle a real path is the handle's `path` and the confined walk.
  A missing path or a dangling symlink is `not_found`, a path through a file `not_a_directory`,
  and a symlink loop `io_error`, as is a real path that isn't UTF-8.
  A relative path is a `TypeError`, as for `open_dir`.
- Paths beneath a handle are relative and `/`-separated, resolved as `openat2` with `RESOLVE_BENEATH` does.
  An absolute path or `..` above the handle fails with `outside_root`,
  and so does an absolute symlink, or a relative one whose target climbs above the handle,
  even when either would land back inside.
- A path's own `..` segments resolve lexically, before symlinks, so `link/..` is the handle's directory.
  The runtime's native op is to normalize them the same way before calling the kernel, a requirement on its wrapper,
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
- A handle is bound to its directory's path, not its inode, on both runtimes.
  When the directory is moved or replaced, the handle follows the path,
  refused when the path now resolves elsewhere (`outside_root`) or nowhere (`not_found`).
- Errors are `FsError`, typed by `kind` (`not_found`, `already_exists`, `permission_denied`,
  `not_a_directory`, `is_a_directory`, `not_empty`, `outside_root`, `not_granted`, `io_error`).
  `not_granted` is a reach outside what the run granted, refused at the call whether or not anything is there,
  so it reveals nothing outside the grants; only the runtime's builtin module throws it, since the Node module checks no grant.
  What a handle may do still tells what it reaches: a handle that only writes learns, from how its writes end,
  whether a name beneath it exists (`create_new`'s `'exists'`, `mkdir`'s `already_exists`, a `remove` or `rename` of nothing),
  what kind it is (`is_a_directory`, `not_a_directory`, a trailing `/`, a symlink out as `outside_root`, a FIFO or device as `io_error`),
  whether a directory holds anything (`not_empty`), whether the process may change an entry (`permission_denied`),
  where another filesystem or mount begins (a `rename`'s `io_error`), and, by the rule a `remove` or `rename` keeps on `fgro` ([fs on fgro](#fs-on-fgro)),
  which directories beneath it are the run's.
  Match one with `is_fs_error(error, kind?)`, which also recognizes one from another copy of Gro.
  Both live in `@fuzdev/gro/fs_error.ts`, apart from the host module,
  so code that only makes or matches an error imports no capability.
  It's part of the host-module surface: on `fgro` it's a builtin module that grants nothing,
  so the builtin module and every importer share one `FsError` class.
  An error's `path` is the absolute path the op was asked for, or for `outside_root` the path as given,
  never a path a symlink led to.
- Each op's one expected failure returns a value instead of throwing:
  `stat` returns `null`, `exists` returns `false`, `read_if_exists` and `read_text_if_exists`
  return `null`, `remove({missing_ok})` and `mkdir({exists_ok})` return,
  and `write({create_new: true})` and `symlink` return `'exists'`.
  Any other failure still throws, so a permission error is never read as a missing file,
  and a path through a file (`f.txt/x`) is `not_a_directory`, not missing, sync and async alike.
- `stat_sync`, `exists_sync`, and `read_text_sync` are the sync ops, for hot build paths.
- `list` yields a directory's entries sorted by name, comparing the names' UTF-8 bytes
  (code point order, never a locale's, and not JS's default sort, which compares UTF-16 code units and puts an astral name before U+E000 to U+FFFF),
  so a listing is in one order on every filesystem and both runtimes.
  It reads the whole directory before yielding the first entry, holding every name in memory.
  A name that isn't UTF-8 fails the listing with `io_error` before any entry, so where it would sort never shows.
- A symlink's target is data, and following it is what's confined.
  `dir.symlink(path, target)` stores any target verbatim, relative, absolute, or dangling, never resolving it,
  and `dir.read_link(path)` returns one as stored, never following the link, so reading it reaches nothing.
  A handle follows a link, its own or anyone's, only when the target is relative and stays beneath it.
  `symlink` takes the path first, like every handle op, and returns `'created'`,
  or `'exists'` for any entry already at the path, a dangling link included, left untouched.
  The kernel's `EEXIST` decides, so of two racing calls one creates and the other gets `'exists'`.
  A path ending in `/`, `/.`, or `/..` and a target that's empty, holds a NUL byte, or isn't well-formed Unicode are a `TypeError`,
  but a path that resolves to the handle's own directory (`''`, `'.'`, `'d/..'`) is an existing entry, so `'exists'`.
  `read_link` of an entry that isn't a link is `io_error`, as is a target that isn't UTF-8.
- `dir.set_times(path, {mtime_ms, atime_ms?}, {follow?})` sets the times in milliseconds, as `stat`'s `mtime_ms`,
  `atime_ms` defaulting to `mtime_ms`, and "now" is the caller's `Date.now()`.
  A time is a finite number from `0` to `8.64e15`, else a `TypeError`, and the filesystem may clamp a time beyond its own range.
  On the Node implementation an integer can come back from `stat` a fraction of a microsecond off in some spans, and more past 2242 ([below](#the-node-implementations-gaps)).
  It follows a final symlink, confined, unless `follow` is `false`, which sets the link's own times,
  an op the run box refuses ([native requirements](#native-requirements)).
- `dir.set_mode(path, mode)` sets the permission bits, an integer from `0` to `0o777`, else a `TypeError`:
  absolute, never symbolic or a string, and never a setuid, setgid, or sticky bit.
  It follows a final symlink, confined, since Linux has no mode for a link.
  `write`'s `mode` takes the same bits, an integer from `0` to `0o7777` masked to `0o777`, before the umask applies.

The module's TSDoc has each op's contract.

### fs on fgro

On `fgro`, `@fuzdev/gro/fs.ts` is a builtin module with the Node module's members, each of the same `length`, its records and functions frozen.
It needs the `fs` grant, which `fgro run` gives the project's own modules, `-A` every module,
and its roots are the directories the run's flags grant ([run](run.md#usage)).
Every member is built.
How each part is built is in the module docs of `crates/gro_runtime/src/host/fs.rs` and its children.

- The run's roots are the directories it reads or writes, opened the first time the run imports `fs`:
  the entry's [project root](glossary.md#project-root), read with no flag, so `fs.project` is that directory, never the working directory;
  each directory `--allow-read=` and `--allow-write=` grant; and under `-A`, `/`, read and written.
  Each directory `fs.temp_dir` makes, with `--allow-temp` or `-A`, joins them as the run goes.
- A flag's directory is the one its path named as the run started:
  one replaced at its real path or gone from it by the first import fails every op through it with `io_error`, its cause saying which,
  and a flag's symlink pointed elsewhere since still names the directory it named.
- Each root reads and writes what its flags grant, joined with what every root holding its directory does,
  so a directory written inside the project is read too, a directory only written is read by no read op (though `fs.open_dir`, `dir()`, and a handle's `path` still tell what's there,
  and, a handle's `path` being its real path, where a directory symlink in it leads),
  and the paths that name one real directory are one root, its grants joined.
- A handle carries a scope, whether it reads and whether it writes, and what an op may do is never read from the run's roots.
  `fs.open_dir` and `fs.project` give a handle its root's scope, `dir()` keeps its receiver's (an `FsReadDir`'s writing nothing),
  and `readonly()` drops the write; nothing widens a scope, so `fs.project.dir('out')` writes nothing even when `out` is granted:
  a program writes a granted directory through a handle `fs.open_dir` makes on it.
  A read op (`stat`, `exists`, the reads, `read_link`, `list`) through a handle that doesn't read is `not_granted`, after its path is checked and before anything is looked up,
  and so is a write op through one that doesn't write, after its paths are checked, and a built one's other arguments.
  A `copy` reads its `from`, so a handle that doesn't read is refused it naming `from`, before the write is judged.
  `set_times`, `set_mode`, and `symlink` each write.
  `dir()` and `readonly()` work on any handle, so a handle that only writes still narrows to a subdirectory.
- `fs.open_dir` and `fs.real_path` take an absolute path inside a root, `fs.real_path` only a root that reads, its `..` resolved as Node's `realpath` resolves it:
  after the symlink before it, so `link/..` is the parent of `link`'s target, and `missing/..` and `a.txt/..` fail as they do on Node,
  and a path that climbs above a root and back in reaches what Node's does.
  One outside every root is `not_granted`, also when nothing is there, and so is one through an absolute symlink,
  even to a path inside its root, or a relative one whose target climbs above its root, even back into it, both of which Node follows.
  A relative target that stays within the root is followed, and the target opened by its own path is reached when a root holds it;
  a grant of the target's directory alone never lets the path through the link, since a root holds a path by its text, not by where a link leads.
  A directory granted inside another refuses nothing the other reached.
  The symlinks before each `..` are counted apart, so a path with more than Linux's 40 across its `..`s resolves where Node's is a loop (`io_error`).
  A root is matched by its real path and by each path it was granted as (but one holding a `..`),
  so `--allow-read=$HOME/.claude` holds the paths beneath it under a symlinked home directory;
  the project's root is matched by its real path only, unless a flag names it too, so a path that reaches it through a symlink outside it is `not_granted` where Node opens it.
  A root at `/`, `-A`'s or a flag's naming `/`, resolves an absolute symlink and a `..` at `/` as the kernel does, so `fs.real_path` and `fs.open_dir` reach what Node's do.
  A `not_granted` is listed and fails the run, caught or not, as `env`'s does ([run](run.md#errors-and-exit-codes) has the hints it prints).
- `fs.real_path` returns a path inside its root; an entry that left the root while it was opened is `io_error`.
- `stat` works on any entry, a FIFO or a socket among them, and never waits.
- A read never waits on its file, and refuses anything but a regular file before a byte is read:
  a directory is `is_a_directory`, as on Node, and a FIFO, a socket, or a device is `io_error`,
  where the Node module waits on a FIFO until a writer closes it, `process.exit` included, and reads a device like `/dev/zero` without end.
  A regular file is read to its end, never trusting the size it reports to end it (a `/proc` file reports 0),
  and one longer than `2 ** 31 - 1` bytes, the most Node's `readFile` reads, or that reports more, is `io_error`,
  a size past it refused before a byte is read, as on Node.
- `read` and `read_if_exists` resolve with a `Uint8Array` of a buffer of its own, which the program owns.
  A buffer V8 can't allocate, after collecting all it can, ends the process as out of memory, and so does `read_text`'s,
  where the Node module's read rejects with `io_error`, its cause Node's `RangeError` (`ERR_MEMORY_ALLOCATION_FAILED`).
  `read_text` decodes as Node's does: each malformed sequence a U+FFFD, a BOM kept.
  As a promise fulfilled with any object does, the bytes' promise looks their `then` up through the realm's prototypes,
  so a `then` a program put on `Object.prototype` runs there, as on Node.
- `list` returns its listing at once and reads nothing, as the Node module's async generator does:
  the first `next()` checks the receiver and the path, reads the whole directory, and sorts it,
  so a missing directory, a path that's no string, or a borrowed receiver rejects that `next()`, never the call.
  A listing holds no open directory, only its entries, so one abandoned, returned from, or thrown into leaves nothing to close.
  A failure ends the listing, as `return` and `throw` do, and every later `next()` is done.
- A listing is a frozen handle of the host's with `next`, `return`, `throw`, and `[Symbol.asyncIterator]`, TypeScript's `AsyncIterableIterator`,
  not an async generator: its results have a null prototype, as `stat`'s record does, its `next` declares no argument,
  and `return(value)` settles with `value` as given, never awaiting it, so a rejected promise given to it is left unhandled, which fails the run. Its entries are plain objects, as on Node.
- An op's error has the Node module's `kind` and `path` and the message `kind: path`.
  Its `cause`, when the OS said something, is an `Error` with the OS's words, which differ from Node's.
- An async op makes its calls inline and returns a promise already settled.
  So `Promise.all` over many ops gets no parallelism, a slow filesystem blocks timers and other work for the op's length,
  and a loop of awaited ops runs on microtasks alone, never returning to the event loop, so a timer waits for the whole loop.
- `stat` resolves with a null-prototype record, where the Node module's is a plain object.
- An option given through a getter or a proxy is a `TypeError`, since an op runs no code of a caller's,
  and so is a time of `set_times`' given so; `times` that isn't an object reads no time, a `TypeError` naming `mtime_ms`, where the Node module's message is the engine's.
- `write` takes a string, written as UTF-8 with each lone surrogate a U+FFFD, or a `Uint8Array`, else a `TypeError`, as on Node.
  It follows a final symlink beneath the handle, as on Node, a dangling one making its target,
  and never waits on its target, refusing anything but a regular file:
  a directory is `is_a_directory`, as on Node, and a FIFO or a device is `io_error`, where the Node module waits on a FIFO's reader and writes to a device.
  The file is emptied as it opens, before the write, so a write that fails can leave it empty or short, as Node's `writeFile` can: no write replaces a file atomically.
- `mkdir` with `recursive` makes each missing directory by its name, passing only a directory, a symlink to one followed,
  so a dangling symlink on the way is `not_a_directory`, where the Node module's confinement makes the path the link names.
- A recursive `remove` removes a symlink inside the tree, never following it, and its error names the path removed, never an entry inside it; a tree deeper than the process may hold descriptors open fails with `io_error`, where Node's removes it.
  A mount inside the tree is emptied as any directory is.
- A `remove` or a `rename` never removes or moves a root's directory, the project's included, or a directory holding one,
  whatever the run grants, `-A` included: it's `io_error`, its cause saying so, before anything is removed or moved, where Node does it.
  A rename's destination is held so too, since a rename over an empty directory replaces it.
  An end is judged by its real path, so no spelling, a symlink on the way among them, reaches around the rule,
  and before the kernel acts, so a `rename` of nothing over a root's directory is this `io_error` too, where Node's is `not_found`.
- A `rename` that would move an entry where the deepest root holding the destination grants what the source's doesn't is `io_error` too, where Node moves it:
  with `--allow-write=/o --allow-read=/o/r`, a handle on `/o` writes but doesn't read, and moving `secret.txt` to `r/secret.txt` would let the run read it.
  A move to where the grants allow less, or the same, is a move, and such a `rename` of nothing is `io_error` too, where Node's is `not_found`.
- A `rename` the kernel refuses between two directories (`EXDEV`, which two mounts make, and so can a sandbox the process inherited) is `io_error`, its cause the kernel's words, where Node's is the same kind.
  Of a missing entry or a non-directory, which could be either end's, the error names the destination when the source is there and the source when it isn't, as on Node.
- `copy` opens both ends beneath the handle, each a final symlink followed, and copies a regular file only, never waiting:
  a FIFO, a socket, or a device at either end, or inside a tree, is `io_error`, where the Node module waits on a FIFO and copies a device.
  What it makes takes its source's permission bits less the setuid, setgid, and sticky bits, as `write`'s mode does:
  a plain copy drops them on Node too, and a recursive one keeps them on Node.
  With `recursive`, a tree is copied as Node's `cp` copies one with `verbatimSymlinks`: a missing parent of `to` made, a directory merged into one there,
  a symlink copied as the link, an entry over a directory or a directory over a file refused `io_error`, and a tree into itself refused, by real paths.
  A symlink copied over a symlink replaces it, where Node's `cp` follows the source link, outside the handle too, and fails when it dangles;
  and a file copied with `recursive` to a dangling symlink makes the link's target, or is `not_found` when the target's parent is missing, where Node's `cp` replaces the link.
  Its error names `from` or `to`, never an entry inside, as on Node.
- A `copy` takes neither half of the rule `remove` and `rename` keep.
  It never removes or replaces a directory, so a root's stays where it is.
  And it is a read and a write through one handle, which must do both:
  the case the scope-gain rule exists for, a handle on `/o` that writes but doesn't read copying `secret.txt` to `r/secret.txt`, is refused its read of `from`,
  and a handle that reads and writes can always copy by a `read` and a `write`.
- `set_times` and `set_mode` act on the entry a final symlink leads to beneath the handle, opened as a path and changed through it, never by a path the kernel resolves again;
  `set_times` with `follow: false` acts on the link itself.
  `set_times` sets a time to the nearest nanosecond, where Node passes seconds as a double its libuv truncates ([`FsTimes`](#fs)).
  `set_mode` on a root's own directory changes its bits and nothing of the roots, as on Node.
- `symlink` makes its link by its name in the parent opened beneath the handle, never following it.
- `fs.temp_dir` needs `--allow-temp` (or `-A`): without it it's `not_granted`, listed, naming the prefix, never the temporary directory, with a hint naming the flag.
  The temporary directory is `TMPDIR`, `TMP`, or `TEMP`, the first set and not empty, else `/tmp`, as Node's `os.tmpdir()` names it, read as the run starts, and only when the run grants it,
  a relative one resolved against the working directory, so an error names it absolute, by its real path, where Node's names it as given.
  It's held to its real path then (where it can't be resolved then, to its path with its `.` and `..` resolved by their text):
  a symlink on that path, at its last name or before it, as a run that writes there can make one since, is `io_error`, never followed, where Node follows it.
  Its directory is made beneath a descriptor on the temporary directory, by a name of the prefix and six random letters from the OS's random source, mode `0o700`, as `mkdtemp`'s,
  and is a root the run reads and writes, which the rule keeps where it is.
  Disposing of the handle (`await using`, or its `[Symbol.asyncDispose]()`) removes the directory and everything in it beneath the root's own descriptor, then retires the root:
  every op through a handle on it or beneath it is `io_error` after, where the Node module's finds nothing there (`exists` false, `stat` null, the rest `not_found`),
  its path is no root, and disposing again does nothing, as Node's forced `rm` does.
  A disposal that fails rejects with an `FsError`, where the Node module rejects with Node's own error, and keeps the root, so a later one can try again.
  A directory never disposed of stays after the run, as on Node.
- A kernel below Linux 5.6, which lacks `openat2`, fails every op alike with a plain `Error`.
- `fs` lands before the [deny overlays](glossary.md#deny-overlays) and before `fgro run` has a kernel box,
  so a program can read the project's `.env` files and any file inside a directory the run reads,
  and write under `.git` and `node_modules` inside a directory it writes.

`fs_search` (`@fuzdev/gro/fs_search.ts`) is library code over a handle,
finding the files beneath a directory through `list`, recursively.
A symlink is a result like a file, never followed, its `kind` telling them apart,
and a missing directory finds nothing.
Its results are sorted by path segment by segment, each compared by code point, the order of its UTF-8 bytes:
`list`'s order taken depth-first, so a directory's files come where `list` puts the directory (`a/z.ts` before `a-b.ts`),
and a search lists in one order on every machine whatever its locale; `sort: false` leaves them in the order found:
each directory's entries in `list`'s order, but sibling directories are searched concurrently,
so the order across directories isn't fixed.
It's a module of Gro's package, not a builtin, so `fgro` loads it from the npm package, granted nothing:
it reaches only the handle it's given, through `stat` and `list`.
Gro's own sweeps, like `gro format`'s, act on regular files only and skip a symlink, as tsv's walk does.

## spawn

```ts
import {spawn} from '@fuzdev/gro/spawn.ts';
import {stdout} from '@fuzdev/gro/stdout.ts';

const git = spawn.program('git', {pass_env: ['GIT_*', 'SSH_AUTH_SOCK']});
const status = await git.out(['status', '--porcelain']);
if (status.kind === 'exited' && status.code === 0) console.log(status.stdout);
await git.run(['fetch'], {cwd: repo_dir}); // `repo_dir` an fs handle, stdio inherited
const commit = await git.out(['commit', '-m', message], {stdin: 'inherit'}); // captured, and may prompt
const blob = await git.out_bytes(['cat-file', 'blob', hash]); // `blob.stdout` a `Uint8Array`
const batch = await git.out_bytes(['cat-file', '--batch'], {
	stdin: new TextEncoder().encode(oids.join('\n') + '\n') // sent while the output is read
});

const dev = await spawn.program('node_modules/.bin/vite').start(['dev']); // stdio inherited, a live child
await dev.closed; // or `dev.kill()`, or `await using` to stop it when the scope ends

const check = await spawn.program('node_modules/.bin/svelte-check').start([], {
	stdio: ['inherit', 'pipe', 'pipe'],
	timeout_ms: 300_000
});
for await (const chunk of check.stdout!) await stdout.write(chunk);
const result = await check.closed;
```

- `spawn.program(name, options?)` makes a `SpawnProgram`, the capability to run one executable,
  resolved once when it's made and pinned for the handle's life.
  `spawn.find(name, options?)` is the same, returning `null` instead of throwing `not_found`,
  and any other failure, like `not_executable`, still throws.
- A program handle bounds which binary runs, not what it does: for most tools the arguments are code
  (`git -c core.sshCommand=…`, `node -e`, `ssh -o ProxyCommand=`, `find -exec`), and a tool reads its own config from the project and `HOME`,
  so on `fgro` a spawn grant is designed to be bounded by the kernel backstop around the child, and the policy diff shows it that way ([backstop](backstop.md#the-box-for-runs-and-children)).
- A bare name walks the host's `PATH` synchronously, absolute entries only, to the first regular executable.
  The child's own `PATH` lookups use the host `PATH` as it is, so a relative or empty entry in it resolves against the child's cwd,
  and its `node_modules/.bin` entries stay, so a child's own lookup, like a `#!/usr/bin/env node` script's, still searches them.
  The walk skips every `node_modules/.bin` entry, one whose last two segments are `node_modules` and `.bin`
  once empty and `.` segments are dropped and `..` resolves lexically, with no symlink resolved,
  so the entries `npm run` and `deno task` put ahead of the rest are never searched,
  and a dependency's `.bin/git` in one can't shadow `spawn.program('git')`.
  An entry reaching a `.bin` directory through a symlink, or a directory a tool fills with bins under another name, is walked.
  A project's tools are reached only by a name containing `/`. Gro's own tasks find a tool the project declares,
  like `eslint` and `svelte-kit`, only there, a builtin through its `host.programs`, whose `local` program grants
  look with `find_local_cli` (`cli.ts`) at the project's `node_modules/.bin/<name>`, and a root with `to_cli_finder`;
  one the project needn't declare, like `vite` and Gro itself, with `find_cli`, which tries that path first
  and then the bare name on `PATH`,
  so a tool installed only in an ancestor's `node_modules`, as a workspace hoists one, isn't found;
  `git`, `node`, and the package manager go straight to `spawn`.
- A name containing `/` resolves against the project root. It's a lexical check:
  a name that's absolute or climbs above the project through `..` is `not_granted`,
  while a symlink beneath the project is pinned wherever it leads, as a `PATH` entry would be.
- `spawn.program_in(dir, name, options?)` makes a handle for a program beneath an fs handle's directory,
  for what a run builds outside the project, like a cargo binary under a cache directory.
  `name` is relative and resolves as `dir` resolves any path, so an absolute name, a climb above `dir`,
  an absolute symlink, or a relative one that climbs out is `not_granted`, the `FsError` (`outside_root`) as its cause's `cause`,
  while a relative link that stays inside, like a `.bin` link, is followed.
  The absolute name and the climb are refused before anything on disk is read.
  `spawn.find_in(dir, name, options?)` returns `null` where `program_in` throws `not_found`
  (missing, a path through a file, a directory, a trailing `/`), and a failure to resolve the name another way,
  like a directory on the way this user can't search, is `not_executable` with the `FsError` as its cause's `cause`, never a miss.
  `dir` must be a handle from this copy of `fs`, read-only or not; a forged object or another copy's handle is a `TypeError`.
  A narrowed root refuses both, `not_granted`, since a set of names can't grant a directory.
  The pin, the script rules, and the environment are `program`'s, and `argv[0]`, and a script's `$0`, is the absolute path it was found at,
  the handle's real path joined with the name, symlinks kept.
- The pin is the executable's real path. A later `PATH` change changes nothing.
  A native binary runs at the real path, so a swapped symlink changes nothing either. A bare name is its own `argv[0]`,
  since a multicall binary like `~/.cargo/bin/cargo -> rustup` picks its behavior from `argv[0]`,
  and a name containing `/` passes its lexical project-absolute path,
  so a tool finding itself through `argv[0]`, like a virtualenv's `python`, still does under another `cwd`.
- A `#!` script runs at the path it was found at, the `PATH` entry's or the lexical project-absolute one, symlinks included.
  The kernel hands the interpreter the path it exec'd as `$0`, dropping `argv[0]`,
  so a script sees the name it was called through, as it would from a shell.
  Debian's `psql`, a link to the `pg_wrapper` script that dispatches on `$0`, runs as `psql`,
  and a `node_modules/.bin` link to a Node script sees the `.bin` path as `process.argv[1]`.
  A script is pinned by identity, the device and inode of the descriptor its first bytes were read from.
  Each launch stats the found path again just before the exec, and one that leads to another file,
  a link retargeted or a file renamed over the pin, throws `launch_failed`, as does one that no longer resolves.
  The same file reached by a new real path, a directory above it moved and linked back, still runs,
  and a script rewritten in place, or deleted and recreated on a reused inode, still matches.
  The interpreter opens `$0` by path after the exec, so a link retargeted before it does runs what it then leads to,
  and in that window whoever can write a directory on the found path can redirect a held handle,
  where a native binary is reached only through its pin's own path. A native binary runs at its pin, unchecked.
  The interpreter a `#!` line names is never pinned or confined: the kernel finds it at each exec,
  so a relative one resolves against the call's `cwd`, and a script beneath `program_in`'s handle can name one outside it.
  `name` and `path` are for display; the pin itself is private, and `path` is the real path for a script too.
- A file is pinned only when its first bytes are a format the kernel runs, else it's `not_executable`,
  an empty file included, since on Linux `execvp` hands any other file the kernel won't run to `/bin/sh`.
  `#!` is accepted everywhere, the ELF magic everywhere but macOS,
  and on macOS the Mach-O magics, 32-bit and 64-bit in either byte order and fat binaries.
  A file with execute permission this user can't read passes, as it can't be inspected.
  Its format can't be told without reading it, so it's taken as native and runs at its pin:
  a native binary runs, and an unreadable script fails at each call when its interpreter can't open it,
  a result from the interpreter, not a `SpawnError`.
  `binfmt_misc` formats are refused along with the rest.
- `program.run(args, options?)` runs with stdio inherited.
  `program.out(args, options?)` captures stdout and stderr as UTF-8 text,
  with `U+FFFD` for bytes that aren't, up to `max_bytes` per stream, 64 MiB by default.
  Output past the cap kills the child with `SIGKILL` and the result has `truncated: true`,
  its `kind` then `signaled` unless the child had already exited,
  and a multi-byte character cut at the end becomes `U+FFFD`.
- `program.out_bytes(args, options?)` is `out` keeping the output as bytes, for what isn't text,
  like a git blob: the same options, cap, stops, and `truncated`, with `stdout` and `stderr` the exact
  bytes the child wrote, each a plain `Uint8Array` holding its own memory, never a `Buffer`, nothing decoded.
- `stdin`, on `out` and `out_bytes`, is `'ignore'` by default, `/dev/null`, or `'inherit'`,
  which shares the host's stdin and keeps the child in the terminal's session, by the grouping rule below,
  so a command that may prompt (ssh, a credential helper, a pinentry through `git commit`) can capture too.
  A `Uint8Array` is sent to the child through a pipe that then closes, at once for empty data.
  It's bytes only, since a string would collide with the mode names, so text goes through a `TextEncoder`.
  It's copied when the option is read, and written while the output is read, so input and output past a pipe's buffer can't deadlock.
  A child that exits or closes its stdin before reading it all ends the write without an error, and the result stands.
  The cap, `timeout_ms`, and `signal` end the call mid-write as they do any other.
- A result is `{kind: 'exited', code}` or `{kind: 'signaled', signal, signal_number}`, so a non-zero exit is a value.
  A signal is named, like `SIGTERM`, and numbered, so a shell's exit code for it is `128 + signal_number` with nothing more imported,
  and a stopped call's result, below, carries both too.
  A program that can't start throws `SpawnError` of kind `launch_failed`, as does a script whose found path now leads to another file.
  A captured call that can't allocate memory for a stream kills the child and rejects with `RangeError` once it has ended.
  `not_found`, `not_executable`, `not_granted`, and `launch_failed` are the kinds,
  matched with `is_spawn_error(error, kind?)`. Both live in `@fuzdev/gro/spawn_error.ts`, apart from the host module,
  so matching one grants nothing; on `fgro` it's designed as a builtin module that grants nothing, resolved beside `spawn`.
- `program.start(args, options?)` resolves to a `SpawnChild` once the child is running,
  its stdio set per stream with `stdio`, one of `'inherit'`, `'pipe'`, `'ignore'` for all or three in order,
  defaulting to `'inherit'`. A launch failure rejects with `launch_failed`, as for `run` and `out`.
  Its `pid` is for display, and nothing in the module takes a pid.
  `closed` resolves to the result, and never rejects.
  `stdin` is a writer or `null`: `write` resolves `false` when the child closed its stdin or ended,
  and `close` ends the stream.
  `stdout` and `stderr` are `AsyncIterable<Uint8Array>` or `null`, read once:
  a second iteration is a `TypeError`, and leaving a `for await` early discards the rest while the child keeps running.
  Each chunk is a plain `Uint8Array` copy. A stream nobody reads buffers to a limit and pauses,
  so a child blocks once its pipe fills, as with any pipe nobody reads.
- `child.kill(signal?)` sends a signal by name from a short allow-list, `SIGTERM` by default,
  and any other name or a number is a `TypeError`.
  It sends once per call, so `SIGTERM` then `SIGKILL` works,
  and returns `false` once the child has ended, sending nothing, so it can't reach a reused pid.
  Disposing the child (`await using`) stops it, `SIGTERM` then `SIGKILL` after a grace period, and waits for `closed`.
- `timeout_ms`, a positive safe integer of at most `2147483647`, and `signal`, an `AbortSignal`,
  stop a call's child on `run`, `out`, `out_bytes`, and `start`: `SIGTERM`, then `SIGKILL` after a grace period.
  The result is `{kind: 'timed_out' | 'aborted', code, signal, signal_number}`, with the `code`, or the `signal` and its `signal_number`, the child then ended by,
  the others `null`, so a child that exits 0 on `SIGTERM` isn't mistaken for one that finished,
  and a call never rejects for it. A `signal` already aborted when the call is made starts nothing
  and throws `launch_failed`, its reason on its cause's `cause`, since a result would claim a child ran.
- A child with no inherited stream leads its own process group, and `timeout_ms`, `signal`, `kill`, and the capture cap reach the group,
  so a stop or a kill sent while it runs reaches what it left in the background, and a stop's `SIGKILL`
  still reaches the group after the leader exits. A captured child leads one unless its `stdin` is `'inherit'`.
  Grouping is a new session (`setsid`), which drops the controlling terminal,
  so a child with an inherited stream stays in the terminal's session and foreground group,
  where `/dev/tty` prompts and Ctrl-C still reach it, and is stopped, and capped, by pid alone,
  so what it started survives a stop.
- After the direct child exits, a call waits a bounded time for its pipes to close, then kills the group, if it leads one,
  and stops reading, so a background process still holding a pipe can't hang the call.
  `out` and `out_bytes` report that with `truncated: true`, since output may be missing from the end,
  whether or not the child led a group.
  A background process that redirected its pipes holds nothing the call waits on,
  so unless a stop has begun, it outlives the call.
  On a `SpawnChild`, a stream nobody reads ends at the bound, its buffered output still readable.
  A reader behind at the bound keeps everything buffered and can read the rest:
  the stream stays paused at its limit, the wait keeping an otherwise idle host alive only until the bound, and ends a bounded time after the reader catches up.
- A child still running when the host exits is killed, `SIGKILL` to its group or its pid,
  by one internal hook installed on the first spawn.
  While a group-leading child runs, a signal that would end the host (`SIGINT`, `SIGTERM`, `SIGHUP`) kills its group first
  and then takes its default action, unless something else listens for that signal, which then decides.
  Another copy of this module doesn't count as something else: each copy kills its own groups, and the signal then takes its default action.
  A `SIGTERM` of the host is also sent on to each child with an inherited stream, under the same rule, so a chain of hosts passes it all the way down.
  `SIGINT` and `SIGHUP` aren't, since the terminal delivers those to its foreground group.
  The hooks are in place before a child is launched, so a signal that arrives while it starts reaches it,
  and a launch that fails keeps them, and the event loop, for two more turns, so a signal that came during it still ends the host.
  Nothing exposes the running children.
- `.bat` and `.cmd` are refused, by the name and by the pin's real path. A call is argv only, never a shell string.
- A name or argument must be a string without a NUL byte or a lone surrogate, else a `TypeError`,
  and a name can't be empty, while an argument can (`git commit -m ''`).
  A lone surrogate would reach a child as U+FFFD, a different string than the caller wrote.
- `cwd` is an fs handle, never a string, and defaults to the project root.
  It must be a handle this copy of `fs` made, as `program_in`'s `dir` must, so a forged object or another copy's handle is a `TypeError`.
- A child starts from a cleared environment, on the runtime's grant rules:
  `PATH`, `HOME`, `TMPDIR`, and `TERM` when the host has them, `LC_ALL=C`, and the host's own
  `NO_COLOR` and `FORCE_COLOR`, then the names `pass_env` lists, then the values `env` gives.
  `pass_env` takes names and `PREFIX_*` globs; `*` alone is refused, since `'all'` says that out loud.
  A glob forwards whatever the host has under its prefix, so `CARGO_*` forwards a `CARGO_TARGET_DIR` too,
  which changes what `cargo` does, as an argument would.
  Environment options exist only on `program`, never per call,
  because a per-call `LD_PRELOAD` or `GIT_SSH_COMMAND` would turn a `git` handle into arbitrary execution.
  The set is fixed when the handle is made.
- `pass_env: 'all'` is the hatch, as strong as `-A`: the child gets the host's whole environment
  and no base pinning (`LC_ALL` included), for forwarding tasks where a person made that choice.
- `omit_env` beside `pass_env: 'all'` leaves out host variables, like the pruning ones an install drops.
  Each entry is an exact, case-sensitive name, never a glob (a `*` is a `TypeError`),
  and giving it with any other `pass_env` is a `TypeError`.
  It changes only what's forwarded, so `env` can still set one of those names.
- `spawn.narrow(names)` is a root limited to `names`, spelled as they'll be requested,
  and it only shrinks: narrowing to a name its parent doesn't allow is `not_granted`.
  A name outside the set is `not_granted` from `program` and `find` alike.
  A narrowed root also refuses `env`, `pass_env: 'all'`, and `pass_env` globs as `not_granted`, through any chain of narrows.
  `env` would turn a named program into arbitrary execution,
  and `'all'` or a set of globs would let the holder of a narrowed handle take the whole environment.
  Named `pass_env` stays, forwarding one host value at a time, so a narrowed caller names each variable.
  To give a library a program with an environment, build the `SpawnProgram` at the root and hand that down.
- To let a library run what it builds, bind the directory at the root and hand down the factory,
  `(name) => spawn.program_in(cache, name, {pass_env})`, so review sees which directory and which environment.
  The factory is the root's authority over that directory, whatever a narrowed spawn handed beside it allows, so it's handed down as a `SpawnProgram` is.
  Holding spawn and a handle runs any program beneath that handle; holding a handle alone runs nothing.

Git is library code over such a handle, not a host module: `git.ts` takes the `git` program
as its first argument and a `cwd` handle in its options, and the root decides its environment.
A root that only reads the local repo, like `gro build`, passes `GIT_LOCAL_PASS_ENV` as `pass_env`.
One that commits or reaches a remote, like `gro publish` and `gro deploy`, passes `'all'`,
since its signing, credential helpers, and hooks are the user's setup and not a closed set of variables,
as strong as `-A` and to be reviewed as such.
A read of the local repo captures with `out`, and anything that may prompt uses `run`,
since a child with no inherited stdio has no terminal for ssh, a credential helper, or a pinentry.
A caller that needs a prompting command's output passes `stdin: 'inherit'` to `out`.

`spawn_restartable` (`@fuzdev/gro/spawn_restartable.ts`) is library code over a `SpawnProgram` too:
it starts a program and restarts it on request, every restart reusing the same handle,
with `args` and `options` copied when it's created,
so the pin and environment fixed at `program()` carry across restarts.
`gro_plugin_server` uses it, pinning `node` once at setup.

The module's TSDoc has each op's contract.

### spawn on fgro

On `fgro`, `@fuzdev/gro/spawn.ts` is a builtin module with the Node module's members, each of the same `length`, its handles frozen,
and `@fuzdev/gro/spawn_error.ts` is a grant-free one, `SpawnError` and `is_spawn_error` the runtime's own ([error classes](#error-classes-on-fgro)).
`spawn` needs the `spawn` grant, which `fgro run --allow-run=` gives the project's own modules for the programs it names, `--allow-run-in=` for the programs found beneath the directories it names,
and `-A` gives every module for every program ([run](run.md#usage)).
Built: every member, the root and its narrowed roots, `program`, `find`, `program_in`, `find_in`, and `narrow`, program handles with their `name` and `path`,
a program's `run`, `out`, `out_bytes`, and `start`, and a started child's `SpawnChild`, its `stdin`, and its output streams.
Not built: a call's `signal` option, which, given, rejects with a plain `Error`.
How it's built is in the module docs of `crates/gro_runtime/src/host/spawn.rs` and its children, `spawn/pins.rs`, `spawn/exec_roots.rs`, `spawn/launch.rs`, `spawn/watch.rs`, `spawn/start.rs`, and `spawn/child.rs`.

- Until each child is held in a box of its own ([native requirements](#native-requirements)), granting any program is close to granting what `-A` grants:
  most programs can be made to run code by their arguments or environment, a shell, git, and cargo among them,
  and the root's `env` values set any variable for a program, `LD_PRELOAD` and `PATH` among them.
  Hand library code a narrowed root, which refuses `env` values.
- The run's root makes only the programs `--allow-run=` names, each by the name a program asks for, and any other name is `not_granted`, where Node's root makes any;
  under `-A` it makes any, as Node's does.
- Each granted name is pinned as the run starts, once the entry has resolved and before any module evaluates, by the Node module's rules,
  a bare name walked on the `PATH` `fgro` read for children, a name holding `/` beneath the project's root,
  its first bytes read through a descriptor on the file it found, which a native binary's pin keeps to run by,
  while a script's keeps none and is run by the path it was found at:
  a call meets what the start found, a fault included, where Node resolves at the call.
  A pin holds the file's identity and its change time (`ctime`) as it was pinned, and a launch checks both first:
  a native binary through the descriptor it runs by, a script at the path it was found at, which must still lead to the pinned file.
  A file rewritten in place, as `fs`'s `write` rewrites one, keeps its identity but not its change time, which no program can set,
  so it's refused, `launch_failed`, as is one whose mode, owner, or links changed, a native binary renamed over or removed among them, said to be replaced or removed when it lost a link;
  a change within the filesystem clock's tick of the file's last change before its pin leaves the change time as pinned, and isn't seen;
  a script whose path now leads to another file is refused too, as on Node, where a native binary replaced at its path would run the new file.
  A granted name that finds nothing is no error until it's asked for, and then `find` returns `null`. Under `-A` a name is resolved when it's asked for, as on Node.
- A child's environment is built when its handle is made, from what `fgro` read for children as the run started, never from the process:
  the base set, then what `pass_env` names, then `env`'s values, and every `PATH` a child gets keeps only its absolute entries,
  the base set's, one `pass_env: 'all'` passes, and one `env` sets alike, where Node passes `PATH` as it is.
  A root lets `pass_env` name only what `--allow-run-env=` forwards: an exact name a pattern covers, a glob only within a glob the flag grants,
  and an exact name of the base set, which every child is given anyway.
  Anything else, `pass_env: 'all'` among it, is `not_granted`; under `-A` any variable passes, `'all'` included.
  `env`'s values are set at the run's root, as on Node, and a narrowed root refuses them.
- A child is started by `posix_spawn`, no code run between its fork and its exec: a native binary by its pin's descriptor, a script by its path,
  so a native child's process name, which `ps` and `pkill` read, is that descriptor's number, where Node's is the binary's name;
  its working directory by a descriptor too, the `cwd` handle's directory opened beneath its root, or the project's, opened as the run started,
  and its environment exactly the program's. Every descriptor `fgro` inherited above stderr is marked close-on-exec as a run that grants a program starts,
  so a child is handed only stdin, stdout, and stderr, and its pipes are pipes, so a child reads `/dev/stdin` as a file.
  That run also raises its soft limit on open descriptors to the hard one, as Node does as it starts, and a child inherits the raised limit; a raise the kernel refuses leaves the limit as it was, as on Node.
  A child inherits the signals `fgro` was started ignoring, as `nohup` leaves `SIGHUP`, and the launching thread's signal mask, the one `fgro` was started with,
  where Node's child starts with every signal at its default and none blocked; a launch that resets both in the child is designed, not built.
- A captured child whose stdin isn't `'inherit'` leads a process group of its own, as on Node, but stays in the terminal's session, where Node's starts a new one:
  such a child that reads `/dev/tty` stops on `SIGTTIN` instead of failing the read, and an `out` with no `timeout_ms` waits for it.
- A child is waited for by its pidfd, which never names another process, and its leader is held unreaped until the call is done with it,
  so a group is killed by an id still its own: at the drain bound, past `max_bytes`, after a timeout's grace, and when the run ends.
  The run ends every child it launched and hasn't reaped, each group a child leads with it, when it calls `exit`, before the process ends,
  and when its event loop returns, whether it drained or an error ended it.
  A `SIGINT`, `SIGTERM`, or `SIGHUP` that ends the run kills the groups first, and a `SIGTERM` is passed on to its children in its own group, as on Node,
  unless a `SIGINT` or `SIGHUP` arriving with it ends the run first.
  The signal's handler waits, for up to a second, for a launch between its child's start and the slot that names the child, so a signal during a burst of launches misses none that arms in time,
  and from the moment it begins every launch is `launch_failed`, its cause saying a signal is ending the run;
  a second signal during the handler waits for it, or runs it again on another thread, rather than end the run mid-way.
  At most `gro_sys`'s caps of children are held at once (1024 group leaders, and 1024 others): past them a launch is `launch_failed`, where Node has no cap.
- A child a real-time signal killed is `signaled`, named by the signal's place after `SIGRTMIN` (`SIGRTMIN+1`), or `SIG32` or `SIG33` for the two the C library keeps below it, and with its own number, where Node reports it as `exited` with code 0,
  or a stopped one as `timed_out` with code 0 and no signal.
  `NODE_V8_COVERAGE` is never added to a child's environment.
- `program_in` and `find_in` find a program beneath an exec root, a directory `--allow-run-in=` names, opened as the run starts, beside the pins, and held to the directory it named then,
  so one replaced since is refused, `not_executable`, its cause saying so, and refuses what it holds rather than yield to an exec root holding it, whose grant may be wider;
  under `-A` every directory is beneath one.
  The handle's directory, by its real path, must lie beneath an exec root of the scope the root names, else the call is `not_granted`, with a hint naming the flag and no path;
  then the directory is opened beneath the exec root's own descriptor, no symlink allowed on the way, and the name beneath it by `openat2` with `RESOLVE_BENEATH`:
  an absolute name, a `..` above the directory, an absolute symlink, and a relative one that climbs out are `not_granted`, as on Node, where a symlink swapped in between the walk and the pin escapes;
  a magic link (`/proc/self/exe`, a `/proc/<pid>/fd` entry) is refused by the kernel as a loop, `not_executable`, where Node's handle refuses one whose link reads as an absolute path `not_granted`, and finds no program at one that reads as no path (a pipe's).
  An exec root stays the directory it opened wherever it's moved: what's found beneath it is found in that directory, never at the path it was granted as now.
  What's found is pinned through the descriptor that found it, as a granted program is: a native binary runs by that descriptor, a script by the path it was found at.
  A program found beneath an exec root may be passed what `--allow-run-env=` forwards.
  An exec root that is also a directory `--allow-write=` grants runs what the run writes there: a run granted both can write a program and run it, so it holds what that program can reach.
- `start` resolves, in a later turn, to a `SpawnChild` once the child is running, each stream as `stdio` says, a child inheriting none leading a group of its own, as on Node.
  Its `closed` never rejects and keeps the event loop running until the child has ended and its pipes closed, or the drain bound passed, as Node's child does;
  a piped output is read from the start, buffering up to 1 MiB ahead of its reader before its pipe is left unread,
  each chunk a `Uint8Array` of its own buffer; a second iteration is a `TypeError`, and leaving a `for await` early, or the iterator's `throw`, drops the rest, the pipe read on.
  A pending `next()`, `stdin.write`, or `stdin.close` keeps the event loop running until it settles, as Node's ref'd streams do.
  `kill` sends its signal by name once to the child or its group, `false` once it has exited; a disposal stops it, `SIGTERM` then `SIGKILL` after the grace, and settles once `closed` has.
  The kill when the run ends, the signal handler's guards, the drain bound, and `timeout_ms` cover a started child as they do `run`'s.
- A refusal by the run's root names its program in the listed refusal and, for a plain name or variable, the flag that grants it in a hint ([run](run.md#errors-and-exit-codes));
  a narrowed root refuses what the Node module's does (a name outside its set, `env`, `pass_env: 'all'`, and a glob) with no hint, since no flag widens it,
  and a variable its program's grant doesn't forward with the `--allow-run-env=` hint, since the flag widens the program's grant.
- A name, an argument, and an option are read as the values they are, converting nothing: an option given through a getter or a proxy is a `TypeError`,
  and an array element given through one reads as none, where the Node module runs it.
- A handle's prototype is the host's record of its methods, enumerable, with no `constructor`, where Node's is a class's.

## The process modules

```ts
import {env} from '@fuzdev/gro/env.ts';
import {args} from '@fuzdev/gro/args.ts';
import {set_exit_code} from '@fuzdev/gro/exit.ts';
import {stdin} from '@fuzdev/gro/stdin.ts';
import {stdout} from '@fuzdev/gro/stdout.ts';

const token = env.get('SOME_TOKEN'); // `undefined` when it's unset
const ci = env.narrow(['CI']); // a handle limited to `CI`

for await (const chunk of stdin) await stdout.write(chunk); // bytes
if (args.includes('--fail')) set_exit_code(1);
```

The running process is these modules, one per kind of authority, each exporting a binding of its name:

| module               | exports                 | what it carries          | grant on `fgro` |
| -------------------- | ----------------------- | ------------------------ | --------------- |
| `env.ts`             | `env`                   | the environment, secrets | `env`           |
| `args.ts`            | `args`                  | the run's inputs         | `args`          |
| `exit.ts`            | `exit`, `set_exit_code` | control over the run     | `exit`          |
| `stdin.ts`           | `stdin`                 | a channel in             | `stdin`         |
| `stdout.ts`          | `stdout`                | a channel out            | `stdout`        |
| `stderr.ts`          | `stderr`                | a channel out            | `stderr`        |
| `cwd.ts`             | `cwd`                   | an observation           | none            |
| `pid.ts`             | `pid`                   | an observation           | none            |
| `runtime_version.ts` | `runtime_version`       | an observation           | none            |

The grants are the design; [the modules](#the-modules) marks which of these are on `fgro`.

Each is its own import, so a module's use of the process shows in its imports, as for `fs` and `spawn`:
one that writes progress imports `stdout` without holding `stdin` or the environment,
and on `fgro`, where there's no `process` global, each is its own grant ([the modules](#the-modules) marks which are built),
so a package can be accepted for one without the others.
No module gathers them, since the loader checks a grant per module when the graph links.
The observations need no grant on `fgro`: each is a value that reaches nothing, and what a package learns from one is little
(the directory the run was started in, where its own `import.meta.url` is already its absolute path, the process's id, and the runtime's version),
so the project's modules and a package's import them alike.
They differ on what a reproducible run can allow (`cwd` can be virtualized, `pid` differs every run,
`runtime_version` keys a cache), so each stays a module of its own, and an import of one is a static fact of the graph
that a hermetic run is designed to act on when it links: virtualize `cwd`, keep `runtime_version`, refuse `pid`.
None of that is built: under `fgro run` each is the real value.

- `env` is an `Env`. `get(name)` returns the value or `undefined` when it's unset,
  and own names only, so `toString` is unset. `get_all()` returns a frozen null-prototype record of the variables,
  which is what a library edge takes, as `load_env` takes a plain record.
  `narrow(names)` is a handle limited to `names`, and it only shrinks.
  A name outside a narrowed set throws `not_granted`, never `undefined`,
  so a denied name can't be read as an unset one, in the same spirit as fs's "a permission error is never read as a missing file".
  A name in the set that's unset reads as `undefined`.
- Names given to `narrow` are exact and case-sensitive, never a glob, and a `*` is a `TypeError`:
  a holder of a narrowed handle sees the names it was given, and review can read them from the call.
  `spawn.narrow` refuses a `pass_env` glob for the same reason.
- An `Env` is read-only and a snapshot taken when the module loads, the runtime's fixed environment.
  A write to the global's `process.env` afterward changes nothing here, as on `fgro`, where there's no such write.
  Spawned children keep reading the live environment when a `SpawnProgram` is made, since that's when the module reads it,
  and restoring a tool's write to `process.env` (as Gro does around `resolveConfig`) still goes through the global.
- A value can be a secret. The module prints none, and its errors name a variable, never its value.
  What a read value becomes is the caller's concern, so hand a library a `narrow` or a plain record of only what it needs.
- `args` are the arguments after the script, a frozen real array, what a script under `gro run` or plain `node` is given.
  In the CLI's own process they're the CLI's arguments, the task name included;
  a task reads its parsed args from its context's `task_args`, since a nested `invoke_task` shares this process.
  With `node -e` the first argument is the first, and a script read from stdin as `-` keeps its arguments too.
- `cwd()` is the working directory when the module loaded, checked as for `fs.project`,
  so importing the module throws in a removed directory or one whose path isn't UTF-8.
  On `fgro` it's the directory `fgro` was started in, its real path with any symlink resolved, and in one whose path isn't UTF-8 the import succeeds and `cwd()` throws when called
  (`fgro` doesn't start in a removed one).
- `exit(code?)` exits, and without a `code` uses what `set_exit_code` set, else `0`.
  `set_exit_code(code)` sets the code the process exits with when it ends on its own.
  A code is an integer from 0 to 255, anything else a `TypeError`, checked with no value of the caller's run,
  so a thenable or a `Symbol.toPrimitive` isn't called.
  `exit` runs the spawn module's child cleanup, like any exit Node runs,
  and it drops writes not yet accepted, so await a write before exiting:
  the CLI's entry awaits an empty write to `stdout` and `stderr` before it exits on an uncaught error.
- `stdin` is an `AsyncIterable<Uint8Array>`, read once, each chunk a plain `Uint8Array` copy,
  a second iteration a `TypeError`. Leaving a loop early stops reading, so a child inheriting stdin gets what the read hadn't taken,
  and the process can exit with stdin still open.
- `stdout` and `stderr` take `write(data)`, a string as UTF-8 or a `Uint8Array`,
  resolving once it's accepted, so awaiting each write paces a writer against its reader, and writes not awaited queue in memory.
  A broken pipe, a task piped to `head`, resolves `false` instead of throwing, and every later write resolves `false` too.
  Data that's neither a well-formed string nor bytes is a `TypeError`.
  Any other write failure, like a full disk, rejects with `StdioError` of kind `io_error`.
  Their type is `ProcessOutput`, declared in `stdout.ts`; `stdin`'s is `ProcessInput`.
- Each stream has `is_terminal`, read from its descriptor without creating the stream.
- `pid` is for display and for naming what only this process writes, like a temporary or lock file;
  nothing in the host modules takes a pid, so holding one reaches no process.
  `runtime_version` is opaque, like `node/v24.14.0`, for display and for keying a cache by the runtime that made it, compared for equality only.
- `env`'s errors are `EnvError` (kind `not_granted`, its `target` the variable), matched by `is_env_error(error, kind?)`
  in `@fuzdev/gro/env_error.ts`; the streams' are `StdioError` (kind `io_error`, its `target` the stream),
  matched by `is_stdio_error(error, kind?)` in `@fuzdev/gro/stdio_error.ts`.
  Both error modules are apart from the host modules, so matching an error grants nothing;
  on `fgro` each is a builtin module that grants nothing, `env_error.ts` built and `stdio_error.ts` designed.
  `args`, `exit`, and the observations throw only `TypeError`.

Each module's TSDoc has its contract.

### The process modules on fgro

On `fgro`, the process modules [the table](#the-modules) marks built are builtin modules, each exporting what its Node module does:
`@fuzdev/gro/args.ts`, whose `args` is a frozen array of strings, the script's arguments, every one after the file `fgro run` was given, as written;
`@fuzdev/gro/exit.ts`, whose `exit` ends the run and whose `set_exit_code` sets the code a run that ends on its own exits with ([below](#exit-on-fgro));
`@fuzdev/gro/env.ts`, whose `env` reads the variables `--allow-env=` grants ([below](#env-on-fgro));
`@fuzdev/gro/cwd.ts`, whose `cwd` is a frozen function returning the directory `fgro` was started in;
`@fuzdev/gro/pid.ts`, whose `pid` is the `fgro` process's id;
and `@fuzdev/gro/runtime_version.ts`, whose `runtime_version` is `fgro/<version>+<commit>`, where Node's is `node/<version>`.
`@fuzdev/gro/env_error.ts`, the env module's errors, is one too ([below](#error-classes-on-fgro)).
A builtin module is no part of the startup snapshot: its exports are made natively, by a script that only asks the host for them,
compiled and run when a run first imports the module,
so a run that imports none pays for none, and `fgro eval`, which imports none, never does.
That first import can come after user code has patched the intrinsics, so the script
names no global but `undefined` and reads no property of an intrinsic, taking what it reads from references the runtime took before any user code ran,
and code that patched `Object` or the prototypes before the import changes nothing the module holds and is never called by it
([native requirements](#native-requirements)).
A builtin module declares no types of its own: its types are the Node module's, and a test holds each one's exports to that module's declarations, the same names, each the same kind of member (a function, data, a handle, a class), each kind of handle's methods those of the Node interface its handles implement, and each function of a `length` its declaration allows, for every builtin module.
A set of cases run under both runtimes holds each builtin module's values to what the Node module's are,
their shapes, property descriptors, and what throws, but where a case names a difference by design.

The observations and `env_error` need no grant: any module of a run imports them, a package's included,
and under `fgro run` each is the real value, with no switch that fixes or refuses one yet.

`runtime_version` carries the commit the binary was built from, as `fgro --version` prints it,
so that builds of different commits never share what a cache keys by the string:
the full SHA, never a prefix of it, since the string is only compared, where a collision costs more than length does.
It's written as semver writes build metadata, and it's a constant of the build, read from nothing when a run starts.
A build whose working tree differed from its commit ends in `-dirty`, like `fgro/<version>+<commit>-dirty`,
which names the commit, not the sources: two such builds of one commit read alike whatever each was built from,
so a dirty build's `runtime_version` isn't an identity, and a cache it keys can be another build's.
A build that embeds no commit, a plain `cargo build` or a `cargo xtask build` of a tree git can't read,
is `fgro/<version>` alone, which every such build of that version shares, so it isn't an identity either.

`args` needs the `args` grant and `exit` the `exit` grant, both of which `fgro run` gives the project's own modules, and the import is checked when the graph links,
for a static import and `import()` alike: a module of an installed or linked package is refused each for now,
naming the package, until `-A` lifts that, which grants every module of the run each builtin module ([run](run.md#usage)),
or the policy file's acceptance does (designed).
So a dependency can't end the run on its own, though one the project hands `exit` to can call it, as with any handle.
`fgro eval` imports no builtin module, the grant-free ones included ([eval](eval.md)),
and an `import()` in code compiled from a string is refused, whatever it names ([run](run.md#what-loads)).
A builtin module's namespace has a `default` export of `undefined`, where a Node module has none.
The other process modules are designed as builtin modules of their own, each with the grant its row above names;
until each lands, its specifier is refused as not on `fgro` yet, in either spelling.
`@fuzdev/gro/process.ts` and `@fuzdev/gro/process_error.ts`, which Gro has none of, are refused in either spelling too,
the message naming the modules to import instead ([resolver](resolver.md#builtin-modules)),
so neither loads from an installed copy of the npm package.

#### exit on fgro

`exit(code?)` ends the process inside the call, as Node's `process.exit` does: no `finally` block runs, no queued job, no timer,
and an error that was unwinding is never reported. What the console wrote before it is whole, since the console writes when it's called.
Its code is the one given, else the one `set_exit_code` set, else `0`, and it reaches the process's exit status as it is, 0 to 255.
`set_exit_code(code)` sets the code of a run that then ends on its own; a run that fails exits 1 whatever was set
([run](run.md#errors-and-exit-codes) has the table, with the rule for a refused `import()`).
The host checks a code as the value it is, a number that is an integer from 0 to 255, converting nothing:
a string, a boxed number, a thenable, and an object with a `Symbol.toPrimitive` are each a `TypeError` with the Node module's message,
the realm's own `TypeError` whatever the global of that name has become, and no property of the value is read.
The code set is the run's state, kept by the host, not the module's.
Nothing is flushed, because nothing is buffered: on `fgro` the console is the only writer to stdout and stderr so far.
On Node, `exit` also runs the spawn module's child cleanup; on `fgro` it kills every child the run started and hasn't reaped,
each group a child leads with it, before the embedder's step ends the process ([spawn on fgro](#spawn-on-fgro)).

#### env on fgro

`env` needs the `env` grant, which `fgro run --allow-env=A,B` gives the project's own modules, for exactly those variables,
and `-A` gives every module, for every variable ([run](run.md#usage)); a package is refused it otherwise, as for `args`.
`fgro` reads the granted variables once, as the run starts, so `env` is the environment then, fixed.
The root `env` reads only the granted names, where Node's reads every variable: an unset one is `undefined`,
and any other name throws `EnvError` `not_granted`, as a name outside a narrowed handle's set does on both runtimes,
the error's message `not_granted: NAME`, never a value.
`get_all()` is the handle's frozen null-prototype record of the variables it may read that are set, the same object each call, in the order `fgro` read them.
`narrow(names)` checks each name in order, a name outside the receiver's set throwing `not_granted` at `narrow`, as on Node, before any `get`.

A handle is the runtime's: an object it makes of the module's prototype, a frozen null-prototype record of one function per method,
holding what the handle may read under a key no program can name, list, or read, then frozen.
A method checks it was called on a handle the runtime made, so an object of the same prototype, a copy, a proxy of a handle,
and any other receiver is a `TypeError`, and `narrow` makes a handle of the receiver's prototype.
Each method reads its arguments as the values they are, converting nothing and running no code of a caller's:
a name is a string, checked as on Node (non-empty, well-formed, no NUL, no `=`, no `*` for `narrow`), with Node's messages,
and `narrow`'s names an array, each element read as its own data.
So, by design, unlike Node: a name list that's a proxy, or an element given through a getter, is a `TypeError`;
the prototype has no `constructor`, so nothing reached from a handle makes another;
and its methods are enumerable, as an object literal makes them, where a class's aren't.
Its methods are frozen, as the prototype is, on both runtimes, since every holder of a handle shares them, so writing a property on one is a `TypeError`.
A name refused `not_granted` fails the run, though the program caught the error, and is listed after it by name ([run](run.md#errors-and-exit-codes)),
so a probe for a variable the run wasn't given can't pass as an unset one;
one the root refused is named in the hint after the list, the `--allow-env=` that would grant it, when it's ASCII letters, digits, and `_`, and one a narrowed handle refused isn't.

#### Error classes on fgro

On `fgro` an error module's class and guard are the runtime's own, made the first time a run needs them, never at a start that doesn't,
and held by the runtime, so the module exports the class its host module's operations throw.
An instance is what the Node class's is: an `Error` (`Error.isError`, a stack, `structuredClone` and JSON as an `Error`'s),
with its own `name`, `kind`, and target, and `cause` when the options give one, whose class and prototype are frozen
and inherit from the realm's own `Error`, whatever the global of that name has become, and which a class of the program's can extend.
Its `message` is `kind: target`, as on Node, and the constructor takes no message ([the modules](#the-modules));
the target is what its module's section says (for an `EnvError`, the variable's name, never its value), and the detail of a failure, when it has one, is its `cause`.
The constructor converts its kind and target to strings, and reads its options' `cause`, as the Node class does,
but a `new.target` that is a proxy (from `Reflect.construct`) has its `prototype` read once more, first, before the arguments are converted,
and a subclass's constructor frames stay in the instance's stack, where Node leaves them out.
The guard (`is_env_error`) recognizes an instance of the class, or of a class of the same name, by its prototype chain, as on Node,
but it runs no code of a caller's: a proxy is no error of the family, nor is a value with a proxy on its prototype chain or on its constructor's,
where Node runs their traps (and throws for a revoked one),
and it reads `constructor`, the constructor's `name`, and `kind` only as data properties, so an accessor there is no match, where Node calls the getter.
The class's and the guard's source text are native code's.
The guard is frozen, as every function of a builtin is, its `length` and `name` not configurable, where Node's isn't.

## net

```ts
import {net} from '@fuzdev/gro/net.ts';

const res = await net.fetch('https://api.example.com/items');

const api = net.narrow({hosts: ['api.example.com']}); // a handle limited to that host
const {fetch} = api; // the bare name, in one module
await use_library({fetch}); // any library taking an injected `fetch`
```

The network is an import rather than a global, and `net.fetch` is the only way to it from a handle.
There's no bare `fetch` export, so a module using the network shows `net` in its imports.

- Any `dispatcher` given in `init` or carried on a `Request` is replaced, so it can't send a request around the check.
  undici's versioned global dispatcher still runs every send, so a proxy from `setGlobalDispatcher` keeps working.
- `net.fetch(input, init?)` takes what `fetch` takes, `string | URL | Request` and an init,
  and is assignable to `typeof globalThis.fetch`.
  It's an own frozen closure, so it destructures and hands down as a bare function,
  and a receiver can't change what it reaches.
  It builds one `Request` from the input, checks that object's URL, and sends from it,
  so a stateful `toString` can't differ between the check and the send.
- `net.narrow({hosts})` is a handle limited to `hosts`, and it only shrinks,
  as [`env.narrow`](#the-process-modules) limits names.
  The root handle allows every host, as the root `Env` allows every name.
  An empty list reaches nothing.
- A host is `hostname`, any port, or `hostname:port`, normalized through `URL`
  (lowercase, IDNA, the IPv4 and IPv6 forms) with one trailing dot dropped, so `example.com.` is `example.com`
  and `127.1` is `127.0.0.1`. A default port is compared as a number, so `example.com:443` covers `https://example.com/`
  and not `http://example.com/`.
  `*` is a `TypeError`, as for `env.narrow`, and so is anything but a host, like a scheme, a path, userinfo, or an empty port.
- Only `http:` and `https:` are sent. `data:`, `blob:`, and every other scheme are `not_granted`, on the root handle too.
- Errors are `NetError`, of kind `not_granted`, its `target` the refused `hostname:port` or scheme and never a whole URL, which can carry a token.
  Match one with `is_net_error(error, kind?)`, in `@fuzdev/gro/net_error.ts`, apart from the host module,
  so matching one grants nothing; on `fgro` it's designed as a builtin module that grants nothing, resolved beside `net`.
  A network failure stays a `TypeError`, as from any `fetch`.
- Redirects are the escape a host check has to close, so a handle follows them itself:
  each `Location` is resolved against its response and checked against the handle's hosts before anything is sent to it,
  and a refused host rejects with `not_granted`, no request sent to it.
  A caller's `redirect: 'manual'` or `'error'` passes through unchanged, so a `manual` caller sees the response and decides.
- Following does what `fetch` does on a redirect, compared against native follow case by case:
  `authorization`, `cookie`, `proxy-authorization`, and `host` are dropped when the origin changes;
  a 301 or 302 POST and any non-GET/HEAD 303 become GET, losing the body and its `content-*` headers;
  a 307 or 308 replays the body, its length kept, and a body given as a stream is a `TypeError` on any status but 303;
  at most 20 redirects are followed; a missing `Location` returns the 3xx response as-is;
  an invalid one is a `TypeError`; `signal`, `credentials`, `mode`, `cache`, and `referrer` carry across hops.
  The final response has its own `url`, and `redirected` is `true` on it and any `clone()` of it.
- An `https` to `http` redirect follows when the host is granted, as `fetch` does:
  a grant is by host and port, not scheme, and the origin changing drops the credentials above.

The module's TSDoc has each op's contract.

## watch

```ts
import {fs} from '@fuzdev/gro/fs.ts';
import {watch} from '@fuzdev/gro/watch.ts';

const crates = fs.project.dir('crates').readonly(); // watching needs only a read-only handle
await using watcher = await watch.dir(crates, {recursive: true}); // resolves once it's watching

for await (const event of watcher) {
	if (event.kind === 'rescan' || (event.kind !== 'delete' && event.path.endsWith('.rs'))) {
		await rebuild(); // events queue meanwhile, and debouncing is the caller's
	}
}
```

- `watch.dir(dir, options?)` watches the directory an fs handle is on and resolves once the watch is in place.
  Nothing already there is reported, so a caller that needs the starting state reads it through the handle.
  It rejects with the `FsError` listing the directory would throw (`not_found`, `not_a_directory`, `permission_denied`, `outside_root`, `io_error`),
  and with one when the directory is removed or replaced before the watch is in place.
- The handle is the grant. A read-only handle is enough, and nothing outside it is watched or reported:
  a narrowed `dir.dir('sub')` sees nothing of its siblings.
  `dir` must be a handle this copy of `fs` made, as `spawn`'s `cwd` must, so a forged object or another copy's handle is a `TypeError`.
- An event's `path` is relative to the handle and `/`-separated, so it goes straight back to the handle's ops.
  `recursive` watches the directories beneath, and without it a subdirectory shows only as its own `add` and `delete`.
- Events are `{kind: 'add' | 'update' | 'delete', path, is_directory}` and `{kind: 'rescan', path}`.
  `update` is a non-directory's contents or metadata changing; a directory's changes show as its entries' events.
  `rescan` says changes beneath `path` may have been missed, `''` for everything, so its state is to be read again.
- A symlink is an entry like a file, `is_directory: false`, never followed, as [`fs_search`](#fs) treats one.
  What it points at isn't watched, so a change there reports nothing, and replacing the link reports the link.
- Events are hints to act on, not a log to replay, on the rules the native watch, inotify per directory, can keep:
  - a rename is a `delete` and an `add`, unpaired, and a file replaced by a rename over it, an atomic save, may be an `update` or a `delete` then an `add`;
  - one directory's events arrive in the order they happened, and across directories there's no order;
  - a directory created or moved in is an `add`, then an `add` for each entry found once its own watch is in place,
    so an entry created and removed before then is never seen, and one created as the watch lands may be reported twice;
  - a directory removed or moved out is a `delete`, and its entries may or may not have their own.
- The watcher is an async iterable, read once, a second iteration a `TypeError`.
  Events not yet read buffer up to `max_pending`, 4096 by default. Past it they're dropped for one `rescan` of `''`,
  as a kernel's queue overflows, and later events are dropped until that `rescan` is read, since reading the tree then sees them.
  A directory that can't be watched, like one this user can't read or one past the system's watch limit, is a `rescan`,
  of the nearest directory the failure names and of `''` when it names none, and changes beneath it may go unreported for the rest of the watch.
- A watcher ends on `close()`, `await using`, leaving its `for await`, or `signal` aborting, which drop the events not yet read.
  A `signal` that aborts before `watch.dir` resolves rejects the call with its reason.
  When the watched directory is removed, moved, or replaced by another directory or a symlink,
  the iteration delivers what was queued and then throws `FsError` `not_found`.
  An open watcher keeps the process alive, as a listening server does, and once it has ended nothing of it does.
- Errors are `FsError` from `@fuzdev/gro/fs_error.ts`, so watch has no error module of its own.

The module's TSDoc has the contract.

## blake3

```ts
import {create_hasher, hash} from '@fuzdev/gro/blake3.ts';

const digest = hash(bytes); // 32 bytes, a new Uint8Array

const hasher = create_hasher(); // constant memory, however long the stream
for await (const chunk of stream) hasher.update(chunk);
const streamed = hasher.finalize();
```

- `hash(data)` is BLAKE3's digest of `data`, 32 bytes. `create_hasher()` makes a hasher for the same digest over input given in parts,
  `create_keyed_hasher(key)` one for the keyed hash, a MAC, of a 32-byte key,
  and `create_derive_key_hasher(context)` one for key derivation in a context string, the derived key being the digest of the key material.
  A hasher's `update(data)` adds to its input and `finalize()` returns the digest of the input so far, leaving the hasher as it was.
- Bytes are a `Uint8Array`, its subclasses included; anything else is a `TypeError`, a proxy of one, an array of numbers,
  another view, and a string among them, so nothing a caller passes is converted or iterated: encode a string first.
  A view is read as it is when the call runs: one of a detached buffer is a `TypeError`, as `@fuzdev/blake3-wasm` throws for one,
  and a fixed-length view a resize left out of its buffer's bounds is empty, where that package throws.
  A context is a string, encoded as UTF-8 with each lone surrogate replaced by U+FFFD, as `TextEncoder` encodes it,
  and a key that isn't 32 bytes is a `RangeError`.
- A hasher holds BLAKE3's state, not its input, and lives as long as it can be reached, with no call to end it.
  It's frozen, null-prototype, and its methods throw a `TypeError` called on any other object.
- The module reaches nothing, so it needs no grant, and on `fgro` the project's modules and every package's import it alike.
  `@fuzdev/blake3-wasm` maps its API onto it through its `gro` export condition, so code hashing through that package,
  fuz_util's `hash_blake3` among it, runs unchanged on `fgro`.

On `fgro` the module is the native `blake3` crate's, and a hasher's state is the host's, dropped once the hasher is collected
and counted toward V8's external memory while it lives, so collections keep pace with it.
Its functions, the hasher's methods among them, are the host's, so their source text is native code's,
and a throw's stack starts at the caller, with no frame of the module's.
Its digests are the Node module's, held to the official test vectors' inputs by the differential cases.

## tsv

```ts
import {create_ignore_stack, format_typescript, parse_imports_svelte} from '@fuzdev/gro/tsv.ts';

const result = format_typescript(source); // {ok: true, code} or {ok: false, error}
if (!result.ok) throw new Error(`${result.error.message}\n${result.error.frame}`);

const parsed = parse_imports_svelte(component); // {ok: true, hasModuleSyntax, imports} or {ok: false, error}

const stack = create_ignore_stack();
stack.push_gitignore('', gitignore);
stack.is_ignored('dist/app.js', false);
```

- `format_typescript`, `format_svelte`, and `format_css` format a source, returning `{ok: true, code}`,
  or `{ok: false, error}` for one that doesn't parse, or, from `format_svelte` alone, that it refuses to print
  because printing would change its meaning (a lone CR inside an in-tag `//` comment).
  `parse_imports_typescript` and `parse_imports_svelte` return a source's module requests in source order,
  each record's `kind`, span, `specifier` (its decoded `value` and span, `null` for a computed `import()`),
  `typeOnly`, `phase`, and `attributes`, with `hasModuleSyntax`, or the same `{ok: false, error}`.
  The results, error texts, and offsets are tsv's binding's, `@fuzdev/tsv`: an error's `message` is one line
  and its `frame` the excerpt and caret under it, and every offset is a UTF-16 one, of the source with
  a leading byte-order mark removed for a Svelte document.
  What a function concludes about the source is returned, never thrown.
- A source is a well-formed string, and options are `undefined`, `null`, or an object.
  TypeScript's functions take `sourceType`, `'script'` or `'module'`, exact when set and otherwise a module retried as a script,
  and the Svelte and CSS functions take no key, but a `sourceType` of `undefined`, so one bag can be passed to any of them.
  Anything else is a `TypeError` of tsv's own text, a key it doesn't take among it, and so is a proxy, refused as options that aren't an object;
  a `sourceType` that's a getter is a `TypeError` of Gro's own text: options are read as data, so no code of a caller's runs.
- `create_ignore_stack()` makes a matcher of gitignore-style layers, pushed shallowest-first, each anchored at a directory
  relative to the stack's root (`''` the root): `push_gitignore`, and the override layers `push_formatignore` and
  `push_prettierignore`, applied after every `.gitignore` one, each popped by `pop_gitignore` and `pop_override`.
  `is_ignored(path, is_dir)` matches a `/`-separated path relative to the root, `is_dir` marking a directory,
  so a pattern ending in `/` applies.
  Precedence is push order: a deeper layer pushed before a shallower one loses to it, where git would have the deeper one win,
  and a debug build of `fgro` aborts on such a push (`tsv_ignore` asserts the order there).
  A stack lives as long as it can be reached, with no call to end it; it's frozen, null-prototype,
  and its methods throw a `TypeError` called on any other object.
- The module reaches nothing, so it needs no grant, and on `fgro` the project's modules and every package's import it alike.
  Gro's own modules format, read import requests, and match ignore files through it, never `@fuzdev/tsv`,
  but for the Node loader's type strip, which `fgro` does natively.

On `fgro` the module is tsv's own crates, `tsv_ts`, `tsv_svelte`, `tsv_css`, and `tsv_ignore`, at the version of the Node package,
and a stack's state is the host's, dropped once the stack is collected and counted toward V8's external memory
by its size and its layers' text, not the rules parsed from it, with nothing subtracted for a pop;
nothing bounds a stack's memory but the process's.
Its functions, a stack's methods among them, are the host's, so their source text is native code's,
and a throw's stack starts at the caller, with no frame of the module's.
A call runs on the run's thread, sized to tsv's stack contract, where input nested past tsv's depth limits is a parse error.
A panic in tsv, which only a bug of tsv's is, ends the process, where the Node binding throws it as an error:
the release build aborts on a panic, and a debug one can't unwind out of the host's callback.
Its results are the Node module's, held to them by the differential cases on Gro's own files and sources of every kind.

## Where to import them

- Import host modules at the [composition root](glossary.md#composition-root),
  the task or entry module where authority enters.
  Inference is designed to propose a grant for every entry whose graph links the importing module,
  whether or not that entry calls the code that uses it,
  so importing at the roots keeps each entry's grants to what it wires, and review reads a few files.
- Library code takes capabilities as parameters, through small dependency interfaces,
  the way a function takes its dependencies.
  Tests then pass real handles on a temp directory or plain-object mocks, with no module mocking,
  and a library given fs needs no policy entry of its own, since handing is delegation, not a new grant.
  It also stays runtime-neutral: a library that imports a host module runs only where that specifier resolves,
  and one that takes a `fetch` or a handle's interface as a parameter runs wherever its caller can supply one.
- Name a handle's type with `import type`, never `import {type FsDir}`.
  Under `verbatimModuleSyntax` the second form keeps the import,
  so the host module still links and the library looks like it reaches fs.
- Hand down the narrowest handle (`dir.dir('sub')`, `dir.readonly()`) rather than the module,
  so narrowing happens at the root, where review sees it.
- Gro's own code imports its host modules relatively (`./fs.ts`), like the rest of its `src/lib`;
  `@fuzdev/gro/fs.ts` is the specifier for every other package.
- A path that arrives from outside, like a CLI arg, a config value, or one of Gro's well-known paths,
  becomes a handle at one boundary, an `FsPaths` (`@fuzdev/gro/fs_paths.ts`).
  A composition root builds one from the root and passes it down,
  `create_fs_paths(fs, {cwd: cwd(), home: env.get('HOME')})`, the working directory and `$HOME` read
  through their host modules for the directories its removals refuse;
  it opens any path, so nothing builds one from a handle.
  It resolves a relative path against `fs.project`, follows the path's symlinks the way `fs.open_dir` does,
  the same inside and outside the project, then hands back `fs.project` for a real path beneath the project,
  or else a handle on its parent. Its `remove_path` and `empty_dir_path` refuse the protected directories
  [below](#gros-guards-on-its-own-destructive-paths).
  Gro's own code reaches every path it names through it, its well-known paths like `build/` and `.gro/` too,
  and resolves beneath a handle only what it got from that handle, like a listing's entries,
  so confinement applies to those.
  Every library module that reaches the filesystem takes an `FsPaths` or a narrow handle as a parameter,
  and only roots import `fs` and build one: the CLI's entry (`invoke.ts`), the builtin tasks that declare no grants,
  genfiles, the default config, and the plugin modules like `gro_plugin_sveltekit_library`.
  A builtin whose task declares `grants` is no root: it imports no host module, as Gro's tasks are exported
  and a granted import would lend its grant to every importer,
  and takes its handles from its `host`, cut from the CLI entry's ([task.md](task.md#a-builtins-host-and-grants)).
  The process modules follow the same rule: those roots and the CLI's bootstrap (`gro.ts`)
  import `env`, `args`, `exit`, the streams, and the observations, and library code takes what it needs as parameters,
  a plain record, a `cwd` string, the CLI args, or a stream,
  so `fs_paths.ts` reads nothing of the process and imports no host module.
  The CLI's entry passes its own `FsPaths` to the config's loading and `invoke_task`,
  which carries it in its options to the tasks a task invokes, never on the task's context
  but in a builtin's `host` when its grants name `fs`, with the `env`, `spawn`, `stdout`, and `stderr` roots it cuts the rest from,
  and the project loader it carries the same way, in the `host` of a builtin that loads the project's modules.
  The rendered `$app/env/public` and `$app/env/private` modules are roots too,
  importing `fs` and `create_fs_paths` to read the `.env` files, and `env` and `cwd`
  for the ambient env and the `FsPaths`' protected directories, when they run, so a module importing `$app/env/*` shows those reads in its import graph,
  while the loader that renders them never loads the host module on its hooks thread.
  Nothing imports Gro's `package.json`, and beside the Node-only bootstrap and loader
  (`gro_helpers.ts`, `loader.ts`, `register.ts`), only `internal/node_platform.ts` reads its own
  module's URL, to find the running Gro: the root passes it down in its `GroPlatform`,
  a task's context holding it as `gro`, a `GroPackage` with its version and package directory.
  A function that reads files outside the directory it's named for takes an `FsPaths` rather than a handle,
  like `library_load_from_repo({fs_paths, git}, repo_dir)`,
  since a repo's `package.json`, `tsconfig.json`, or `.gro/` can be a symlink out of it,
  which a handle on the repo would refuse.
  Where a directory's files come from its own listing or search, Gro opens the directory once
  and reads and writes them beneath its handle.
- The boundary's machinery (`internal/fs_boundary.ts`) resolves symlinks through the `Fs` it's built from,
  `fs.real_path` and a handle's `read_link`, so an `FsPaths` reaches nothing that `Fs` doesn't.
- Gro's library and task code otherwise reaches the filesystem only through the fs host module,
  and starts programs only through the spawn host module, so it can run unchanged where the modules are builtin modules.
  The rest that stays on `node:fs` is Node-only machinery: the loader and the modules it runs in its hooks
  (`resolve_specifier.ts`, `svelte_config.ts`, `sveltekit_shim_app_env_vars.ts`), the Filer family and the Node module graph,
  `spawn.ts`'s `PATH` walk and pin,
  the check of the working directory that `fs.project` and `paths` load from (`internal/cwd.ts`),
  the bootstrapping that finds the Gro install to spawn `node` with (`gro_helpers.ts`),
  and the sync stats behind the task-path hints (`internal/task_hints.ts`),
  which the synchronous args parsing calls, where no sync boundary opens a path from outside.
  `spawn.ts` is the one module on `node:child_process`.
  Gro's code formats, reads import requests, and matches ignore files through the tsv host module,
  and the Node loader's type strip (`loader.ts`) is the one module importing `@fuzdev/tsv` itself.
  Gro's code reads the process through the process modules, except what stays on the global,
  Node-only like the fs machinery above: `process.on`, `once`, and `kill`, which no module has,
  so `invoke.ts`'s handlers and `spawn.ts` use them directly, and `spawn.ts`'s live read of the
  environment for a child; the module implementations (`env.ts`, `args.ts`, `exit.ts`, `stdin.ts`,
  `internal/process_output.ts`, `pid.ts`, `runtime_version.ts`, `internal/cwd.ts`);
  and what runs in the loader's hooks or restores a tool's write, `svelte_config.ts`
  (`NODE_ENV`, `MODE`, and the cwd) and `sveltekit_shim_app_env_vars.ts` (the cwd).
  `fgro` has no `process` global, so these move when Gro runs on it.

- Advisory grants, designed and not built, will count a `node:*` import on the Node Gro as the host module it corresponds to,
  and report one with no entry here as ambient:

  | Node builtin                                                               | host module      |
  | -------------------------------------------------------------------------- | ---------------- |
  | `node:fs`, `node:fs/promises`                                              | fs               |
  | `node:child_process`                                                       | spawn            |
  | `node:process`                                                             | the process modules |
  | `node:net`, `node:http`, `node:https`, `node:dns`, `node:tls`, `node:dgram` | net              |

## Hardening

### The module's invariants

The fs module keeps these in the Node implementation, and the runtime's builtin module is designed to keep them too, making its confinement decisions natively ([native requirements](#native-requirements)):

- Handles come only from `fs` and other handles.
  Each constructor checks a key only the module holds, so a handle's `constructor` makes nothing.
- A handle's state lives in private fields, and every op reads it from there,
  so an op called on a borrowed receiver, any object but a handle, throws `TypeError`.
- Handles, their prototypes and the methods on them, `fs` and its functions, and `FsError` with its prototype are frozen,
  so no holder of a handle leaves a value on a method another holder calls,
  and so is `list`'s own `prototype`, from which every listing's iterator inherits.
  Handles, `fs`, an `FsPaths`, and the opened paths it returns are null-prototype,
  so none of them is thenable, even with a polluted `Object.prototype.then`.
- `remove` and `rename` refuse the handle's own directory.
- A path's `..` segments resolve lexically, before any symlink, and never climb above the handle.
  A path is a string without a NUL byte or a lone surrogate, which would encode as U+FFFD
  and name a different entry than it spells, else a `TypeError`.
- An absolute symlink is `outside_root`, and so is a relative one that climbs above the handle,
  even when it would land back inside. A component a path passes through must be a directory.
- `fs.temp_dir` takes only a string prefix, so no `toString` of the caller's names the directory.
- `set_mode` refuses a mode outside `0..0o777`, and `write` masks its `mode` to `0o777`,
  so no op sets a setuid, setgid, or sticky bit.
- `set_times` reads each time once and refuses anything but a finite number from `0` to `8.64e15`,
  so no value becomes "now" or "unchanged" on the way to the kernel.
- `symlink` checks its target as a path is checked, a non-empty string without a NUL byte or a lone surrogate,
  and never resolves it, so creating a link reaches nothing beyond its parent directory.

### spawn's invariants

- Handles come only from `spawn` and other handles, through a module-private key,
  and hold their pin, the path they run at, `argv[0]`, and environment in private fields,
  so a method called on any other object throws `TypeError` and changing `path` changes nothing that runs.
- Handles, their prototypes and the methods on them, `spawn`, results, and `SpawnError` with its prototype are frozen.
- Handles, `spawn`, and a `spawn_restartable` are null-prototype, and a result is a null-prototype record,
  so a polluted `Object.prototype.then` isn't consulted when one is awaited.
- A `SpawnChild`, its stdin, its output iterables, and their iterators are frozen, with the methods on their prototypes,
  and hold their stream and child in private fields, so a reader can't reach the stream underneath.
  Their prototypes are frozen and null-prototype, so none of them is thenable, even with a polluted `Object.prototype.then`.
  Iterator results are null-prototype records and chunks plain `Uint8Array`s, never a `Buffer`,
  and a chunk never passes through promise resolution bare, so a polluted `then` can't replace one on its way to the reader.
- `kill` refuses anything but a name on its list, and a pid goes nowhere, so a `SpawnChild` can't signal another process.
- A child's environment is a null-prototype object, since Node enumerates it with `for in`
  and `Object.prototype.X` would otherwise reach the child.
  The options to Node are null-prototype too, on top of Node's own null-prototype copy of them.
  `process.env` is read through own properties only, so a polluted or inherited name is no variable.
- Each option's value is read once and checked, before the name resolves, then only the checked copy is used.
  Only own properties count as options, so a polluted `Object.prototype.pass_env` sets nothing on a plain `{}`.
- Every array argument (`args`, `narrow`'s names, `pass_env`, `omit_env`) is read by index with its length read once.
- `stdin` data is copied when the option is read, a plain `Uint8Array` only the call holds, so no later write to the caller's array reaches the child.
- `out_bytes`'s `stdout` and `stderr` are plain `Uint8Array`s of their own, copied out of the chunks into memory of their exact length,
  so no `Buffer` or view of memory Node reuses reaches a caller, and they reach it inside a null-prototype record.

### The process modules' invariants

- Handles come only from `env`, `Env.narrow`, `stdin`, `stdout`, and `stderr`, through a key private to their implementation,
  and hold their state in private fields, so a method or getter called on any other object throws `TypeError`.
- The handles, `get_all`'s records, their prototypes, and the methods on them are frozen,
  and every prototype is null-prototype, so none of them is thenable even with a polluted `Object.prototype.then`.
  `args` is the exception, a frozen real array, so its prototype is the unfrozen `Array.prototype` and awaiting it adopts a polluted `then`.
  `exit`, `set_exit_code`, and `cwd` are frozen functions, no constructors, taking no receiver.
- An environment read goes through a null-prototype snapshot holding only `process.env`'s own names,
  so an inherited or polluted name reads as unset.
- `narrow` reads its array by index with its length read once, and a name it doesn't allow throws, never `undefined`.
- An exit code is checked by `typeof` first, so nothing the caller passed runs before the refusal.
- A chunk from `stdin` is a plain `Uint8Array` copy, never a `Buffer` or a view of Node's memory.

### net's invariants

- Handles come only from `net` and `narrow`, through a module-private key, and hold their hosts in private fields,
  so `narrow` called on any other object throws `TypeError`. `fetch` is a closure, so a receiver can't reach or replace a handle's hosts.
- `net`, a narrowed handle, its `fetch`, their prototypes, and the methods on them are frozen, and the prototypes are null-prototype.
- One `Request` is built per call, its URL read back from that `Request`, so no caller `toString` runs between the check and the send.
- Every redirect's target is checked before it's sent to, in the one loop `fetch` goes through unless the caller chose `manual` or `error`.
- `narrow` reads `hosts` by own property and by index with its length read once, and a host it doesn't allow throws `not_granted`.
- Every send names its `redirect` and dispatcher, and the request's own fields a polluted key would replace,
  since undici's init dictionary reads a key the init doesn't own through `Object.prototype`, even for a null-prototype init.
  It's no defense against pollution generally, as the [gaps](#the-node-implementations-gaps) for net say.

### watch's invariants

- Watchers come only from `watch.dir`, through a module-private key, and hold their state in private fields,
  so a method called on any other object throws `TypeError`.
- `watch` and its `dir`, watchers, their iterators, their prototypes, and the methods on them are frozen, and the prototypes are null-prototype.
- Events and iterator results are frozen null-prototype records, so a polluted `Object.prototype.then` isn't consulted on the way to the reader.
- The handle is checked against the handles this copy of `fs` made, read once, for its `path`,
  and its directory listed through the handle before anything watches it.
- Options are read once, by own property.

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

- Emptying or removing a path from outside refuses `/`, the project, `$HOME` as given and as its real path,
  and an ancestor of either, and an `FsPaths` over another project still refuses the working directory
  its root passed, the one Gro loaded in, and its ancestors.
  The first removal or emptying reads `$HOME`'s real path through `fs.real_path`,
  and a failure to read it, but for a missing `$HOME`, fails that removal and every later one
  through that `FsPaths` before it acts.
  So on `fgro`, building an `FsPaths` with `env.get('HOME')` takes `--allow-env=HOME`,
  and a confined removal through it takes `--allow-read` on `$HOME`, or on its nearest existing ancestor
  when it's missing: a `$HOME` outside every grant is refused, missing or not.
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

Where a gap below says what the runtime does, that's its design, not yet built.
The types are the runtime's, and the Node implementation runs the same ops,
but Node can't enforce what the runtime will.
It documents each gap rather than weakening the API to match, and is a review signal, not a boundary:

- Confinement is a check, then an open: a path is walked beneath the handle's directory,
  reading each symlink, then the op runs. A symlink swapped in between escapes, since Node has no `openat2`.
  Recursive `copy` and `remove` hand the whole tree to Node, which walks it by path.
  `read_link`, `set_times`, `set_mode`, and `symlink` run by path too, so a swap retimes, rechmods,
  reads, or links through what the swapped-in link leads to.
- `set_times` hands Node seconds as a double, which libuv truncates to a step:
  a microsecond on libuv 1.51 (Node 24.14), a nanosecond on 1.52.1 (Node 26.13),
  and the implementation treats any libuv below 1.52 as the microsecond step.
  The seconds aim half a step past the time, so the truncation lands on the step nearest it
  wherever the double's spacing is finer than the step.
  On the microsecond step an integer millisecond is stored exactly through 2242
  (`2 ** 33` seconds), and any other time within a microsecond.
  On the nanosecond step a time is stored within half the double's spacing, plus half a nanosecond:
  exactly before 1970-04-08, within 119ns from 2004 to 2038, 238ns to 2106, and 477ns to 2242.
  `stat` shows an integer millisecond exactly but where the seconds' double is coarser than the milliseconds',
  from each power of two seconds to the next power of two milliseconds,
  such as 2004-01-10 to 2004-11-03 and 2038-01-19 to 2039-09-07,
  where about half come back one step of their own double off,
  and from 1970-04-08 to 1972-02-16, where some come back a nanosecond or a few off.
  Past 2242 the double itself is the limit on either step.
  The runtime's builtin module stores a time exactly.
- Intrinsics stay pollutable. Code that patches `String.prototype`, `Array.prototype`, `Promise`,
  `Error`, `TextDecoder`, or `Reflect` after the module loads runs inside its ops,
  and the module blocks only the forgeries named above.
- A handle opened on `/proc` follows its magic links (`/proc/self/fd/1`, `/proc/self/root`),
  which the kernel resolves without a path the walk can check.
- `fs.open_dir` and `fs.real_path` take any absolute path, checked against no grant,
  so no op throws `not_granted`, which the runtime's throws for a reach outside the run's grants.
- Any module can import `node:fs` and the `process` global stays,
  so a narrowed handle bounds only the code that uses handles.
- `fs.project` is the working directory when the module loads, as for the cwd host module's `cwd`,
  so importing the module throws when the working directory has been removed.
  The runtime's is the entry's [project root](glossary.md#project-root), and spawn's project root with it,
  so the working directory carries no authority.
- `process.on`, `once`, and `kill` have no equivalent in the process modules,
  so the CLI's uncaught-error and output-error handlers and the spawn module stay on the `process` global.

### spawn

- A native binary's pin is a path, resolved and then exec'd by path when called,
  so a file replaced at the pinned path in between runs instead.
  A script's is its identity, so a replacement is refused, but a script rewritten in place keeps its inode and runs as rewritten.
- A `cwd` handle follows its path, as an fs handle does. Where `fs` refuses a directory that was replaced
  (`open_dir(mv)`, then a rename and a symlink at `mv`, so `mv.exists('.')` is `outside_root`),
  a call with that `cwd` runs in what the symlink leads to.
- No policy check limits which programs a handle reaches or what they do,
  and no kernel confinement like Landlock applies to a child,
  so a running program reaches whatever its user can.
- A name's `/` confinement checks the name's spelling, so a symlink under the project reaches outside it.
- `program_in` and a call's `cwd` tell a real handle by a registry in `internal/fs_handles.ts`, which the package doesn't export,
  but code importing that file by its path can register any object with any resolver, run what it names, and start a child in any directory.
  The runtime's builtin module tells a handle by the brand the host gave it.
- `program_in` resolves beneath its handle by fs's walk, a check then a use, so a symlink swapped in before the pin escapes the handle,
  as for any fs op, and a native binary it pins runs by path, as above.
- The refusal of a file with no `#!` reads only its first bytes, so a file with a native magic the kernel won't load
  or a `#!` line naming no interpreter or one too long for the kernel's buffer, still reaches the Linux `/bin/sh` fallback of the `execvp` Node starts children with, and a Java class file passes on macOS, sharing the fat binary's magic.
- A child killed by a real-time signal is reported as `{kind: 'exited', code: 0}`, since Node passes an empty signal name for one.
- Intrinsics stay pollutable. The module copies at load the Node functions it calls,
  `Array.isArray`, `Number.isSafeInteger`, the `process` functions, the timers, and `Object`'s `assign`, `create`, `freeze`, `keys`, and `hasOwn`,
  and its own objects are null-prototype, but code that patches `Array.prototype`, `Set.prototype`, `String.prototype`,
  `Promise`, `Uint8Array`, `TextDecoder`, `Error`, `EventEmitter.prototype.on`, or the stream classes after the module loads runs inside its ops,
  and a patched `on` or stream method holds the raw `ChildProcess` or stream.
- Node copies a set `NODE_V8_COVERAGE` into every child's environment.
- Piped `stdin` data reaches the child on a socket, as Node's stdio pipes are socketpairs,
  so a child reopening it by path, like `cat /dev/stdin`, fails with `ENXIO`, where a native builtin module using a real pipe wouldn't.
- A polluted `Object.prototype.encoding`, `highWaterMark`, `objectMode`, or `signal` makes Node's spawn throw after the fork, wrapping the child's streams,
  so the call rejects `launch_failed` with the child untracked and running,
  and with `stdin` data a child reading its stdin never gets EOF and keeps the host alive.
- Grouping needs Node's `detached`, a `setsid`, so a group-leading child has no controlling terminal:
  a program prompting on `/dev/tty` under `out` or `out_bytes` without `stdin: 'inherit'` can't, and fails rather than prompting.
- A child with an inherited stream isn't grouped, so a stop or a kill reaches it by pid alone and what it started survives.
  An inheriting child relies on the terminal for a signal that ends the host, so a `SIGTERM` sent only to the host leaves it running,
  and the exit hook only sees exits Node runs, so a `SIGKILL` of the host leaves every child behind.
- `SIGQUIT` isn't hooked, since a listener would move it to the event loop and a host stuck in synchronous code
  could no longer be dumped by it, the signal's use, so a `SIGQUIT` of the host leaves a group running, as `SIGKILL` does.
- The signal hook defers to any other listener on the signal, so a library that likewise stands down
  when it sees another listener (`signal-exit`'s check) can defer to the hook while the hook defers to it, and the signal ends nothing.
  A `once` listener prepended after a group started has removed itself before the hook counts it, so the hook decides.
- Node can't read a signal's disposition, so while a group-leading child runs, a `SIGHUP` a `nohup` host would ignore
  reaches the signal hook, kills the group, and ends the host.
- A process group is signaled after its leader is reaped, by the drain bound, a stop's grace kill, and the exit and signal hooks,
  so in the moment after the last member exits, the id could be reused by a process the kernel gave it after wrapping every pid.
- The signal hook runs on the event loop, so a host stuck in synchronous code delays the kill of a group until it yields.
- `spawn`, like `fs`, reads the project root when the module loads, so importing it throws
  in a removed or non-UTF-8 working directory.
- Any module can import `node:child_process`, so a narrowed `spawn` bounds only the code that uses it.

### The process modules

- The global `process` stays, so any module can read `process.env` or call `process.exit`,
  and a narrowed `Env` bounds only the code that uses it.
- The root `Env` allows every name. The runtime's holds only the granted names, and a name outside them throws `not_granted`.
- The snapshot is taken when `env.ts` loads, so a write to `process.env` made before that shows.
- `args` under `node -e` and `node -p` depends on the undocumented `process._eval`, since no documented API says the process has no script.
- `exit` in a Node worker ends the thread rather than the process.
- On POSIX Node's stdio writes to files and terminals block the event loop until the kernel takes them,
  where the runtime's complete on their own schedule. Writes to pipes are asynchronous.
- Node's stream `write` is the stream's own, and the streams are created on first use, so code that patched `write`
  or replaced `process.stdout` by then runs inside a write. The modules' other Node functions are copied at load.
  `EventEmitter.prototype` and the stream classes aren't, so code that patches them runs inside these ops holding the raw stream.
- A chunk read from stdin passes through Node's iterator, so a polluted `then` can replace it.
- `cwd`, like `fs`, reads the working directory when the module loads, so importing it throws
  in a removed or non-UTF-8 working directory.

### net

- The hostname is checked, not the address it connects to, so a granted name resolving to a private range, or rebinding between requests, isn't refused.
- The global `fetch` stays, and any module can import `node:http` or `node:net`, so a handle bounds only the code that uses it.
- The root handle allows every host, where the runtime's holds only the accepted ones.
- Following relies on Node exposing a `manual` response's status and `Location`, where the Fetch standard filters them behind an `opaqueredirect`.
- A dispatcher placed at undici's global key reroutes every send, as for the global `fetch`, and a missing one rejects rather than sends past it.
- Beside native follow, following differs in these ways, each for the reason given:
  - a response after a cross-origin hop has `type` `basic`, not `cors`, and a `no-cors` one isn't opaque, since each hop is its own request;
  - a `Request` carrying a stream body is buffered and replayed, where native follow rejects, since a `Request`'s source isn't observable from JS.
    It buffers whether or not a redirect comes, so a large upload streams through `init`, where a stream is refused as native follow refuses one;
  - a redirected request with `integrity` rejects unless every hop's body matches, since the integrity check runs on each hop's response, not only the last;
  - a `Location` with a scheme other than HTTP(S) is `not_granted` rather than a `TypeError`;
  - a redirect's own `Referrer-Policy` isn't applied to the next hop, since no consumer needs it, while the caller's own `referrer` and policy carry.
- Intrinsics stay pollutable. `fetch`, `Request`, `Response`, `Headers`, `URL`, and `Object`'s functions are copied at load,
  so a `fetch` replaced before the module loads is the one called, but code that patches their prototypes after the module loads runs inside these ops,
  and a `Response` is a real one, so awaiting it adopts a polluted `then`.
- `Object.prototype` stays reachable. undici's init dictionary reads a key the init doesn't own through it, and undici's `Client` and `net.connect` read their connection options through it,
  so a polluted `Object.prototype` can change the connection, including its destination, through options like `socketPath` and `lookup`.
  That is part of this module being advisory, as any holder of a handle can already call the global `fetch` or `node:net`, and no list of polluted keys could be complete.
  Every send names `redirect` and the dispatcher, so an ordinary option or a `Request` can't reroute or follow.

### tsv

- The module is over the N-API binding, `@fuzdev/tsv`, which loads a native addon through `node:module`,
  and reads its options as data before handing the binding a bag of its own,
  so a proxy and a getter are refused on both runtimes, where the binding alone would run them.

### blake3

- The module is over `@fuzdev/blake3-wasm`, so a hasher's state is in the WASM module's memory,
  released when the hasher is collected, by the package's `FinalizationRegistry`, and each input is copied into that memory to be hashed,
  where the builtin module reads it in place.

### watch

- `watch.dir` tells a real handle by the registry in `internal/fs_handles.ts`, as `spawn` does,
  so code importing that file by its path can register any object and watch the path it names.
  The runtime's builtin module tells a handle by the brand the host gave it.
- The watch is bound to the directory's path, as an fs handle is, so a moved watched directory ends the watch as a removed one does.
- Chokidar checks an entry with `lstat` and then watches and reads it by path, so a directory swapped for a symlink in between is watched through the link.
  The watcher checks each path before chokidar descends into it and before reporting it: the root still the directory the watch started on (by device and inode),
  and each directory on the way still a directory, not a symlink.
  That's a check then a use, as fs's walk is, so a swap between the two escapes,
  and chokidar can be left watching the directory a swapped-in link points at, never beneath it,
  and the directories beneath one renamed away for the link, their events dropped.
- The root is told apart by device and inode, which a filesystem can reuse: on ext4 a watched directory removed and another made at once at its path usually gets its inode,
  and the watch then goes on in the new one, the directory the handle's path names, before the watch is in place or after.
- An entry replaced at once by one of another type can go unreported, with no `rescan`:
  a file replaced by a directory reports nothing, not even the directory's entries later, and a directory replaced by a symlink is an `update` of the link.
- Names are decoded as UTF-8 by Node, so a name that isn't UTF-8 may go unreported, or arrive with `U+FFFD` in place of its bytes,
  naming no entry the handle reaches, and listing its directory through the handle fails with `io_error`.
- Node exposes no kernel overflow, so a `rescan` comes only from `max_pending` and a directory that couldn't be watched, never from events the kernel dropped.
  A directory that can't be read when the watch starts stays unwatched after it's readable.
- Any module can import `node:fs` and watch any path, so a handle bounds only the code that uses it.

## Native requirements

What the runtime's builtin module must do that the Node implementation can't.
Each handle holds a native resource of its own, and every confinement decision is the host's.
The builtin module's JS reads nothing and decides nothing: each export is a function or a value the host makes natively,
a function checking its arguments and raising the errors, as each builtin's are,
and whatever has identity or state, a class, a brand, a registry, is the host's.
So a function's source text is native code's, and a throw's stack starts at the caller, with no frame of the module's.

That JS is one script the runtime makes from the module's row, the same for every module but whether it hands the host the realm's `Error`,
and it runs when a run first imports the module, which can be after user code has patched the intrinsics,
where the Node implementation copies what it needs when it loads and can be loaded first.
So the script reaches intrinsics only through the runtime's own references to them, taken before any user code ran:
it names no global, calls nothing but the host, reads only what the runtime handed it, and hands on only what the host returns,
and what that hands to user code is made natively: a frozen function, a frozen array, a number or a string, a class, or a frozen null-prototype record.
A test holds every module's script to its template, byte for byte,
and every builtin module is imported by a test in a realm whose every reachable intrinsic reports its use,
where each function it exports or reaches is called with hostile arguments: a proxy, a callback that reports its call, an object of getters, a subclassed string and array.
It shares no source with the Node implementation: the cases run under both runtimes hold the two to one behavior.
A builtin module's script runs the first time a run imports it, and an `import()` runs it on the caller's stack,
so a first `import()` started with less than 96 KiB of stack left runs the script from the event loop instead, with nearly the whole stack,
and settles from the event loop, after the current turn's microtasks and before a 0 ms timer, where one started shallower settles in the turn;
every import of the module gets the one module, however the first ones interleave.
Nearest the stack's limit, as for any `import()` there, the import can reject with the engine's own `RangeError`,
which can also leave a promise the program never saw rejected and unhandled, failing the run.
A builtin module throws nothing as it loads, so nothing it does then is left half-done, or done twice to any effect:
what can fail for a reason of the run's fails in what the module exports, as `cwd` does when called.

### fs

- Make each confinement decision in one op, `openat2` with `RESOLVE_BENEATH` beneath the handle's directory,
  opened beneath its root's descriptor, never a check followed by an open (built).
- Make `fs.project`, and spawn's project root, the entry's project root, never the working directory,
  so the working directory is a string with no authority (built for `fs.project`).
- Pass `RESOLVE_NO_MAGICLINKS` explicitly, so a handle on `/proc` refuses its magic links (built).
- Copy and remove a tree recursively beneath the handle's directory, each entry opened relative to its parent's
  (built for `remove` and `copy`).
- Refuse, in the remove ops themselves, to remove `$HOME` and the working directory, each resolved by the host
  as the run starts, as the roots' rule already refuses the project's directory (designed). The read grant on `$HOME`
  that `FsPaths`' guard takes on `fgro` goes only once its boundary stops resolving `$HOME` itself.
- Check `write`'s data is a string or a `Uint8Array`, and mask its `mode` to `0o777`,
  refusing one that isn't an integer from `0` to `0o7777` with a `TypeError`, as the Node implementation does (built).
- Read a link from its own descriptor, opened as a path only beneath the handle without following it,
  by `readlinkat` with an empty path, so the link read is the one the open resolved (built).
- Check `open_dir`'s and `real_path`'s absolute paths against the run's grants (built).
  Landlock doesn't gate resolving a path, which reads only metadata.
- Create a link with `symlinkat` on the parent's descriptor, never checking the target, which is data (built).
  Landlock's `MAKE_SYM` limits where links are made, the write roots, and Landlock's own walk refuses to follow one
  out of a granted hierarchy for the confined process and its children;
  a link a later unconfined tool follows is the time-shifted case [backstop](backstop.md#the-box-for-runs-and-children) accepts for writes.
- Set times and mode, following a final symlink, on an entry opened as a path beneath the handle,
  through its `/proc/self/fd` link (built).
  Landlock gates neither at any ABI, so the run box has to admit them by form (designed, and open:
  the descriptor forms `utimensat(fd, NULL, …)` and `fchmod(fd)`, which seccomp could pin, aren't the built one), bounded by what the process can open:
  code that escapes V8 can retime and rechmod any file it can open, every read root included, not only the write roots.
  Bounding them to the write roots would take a broker outside the box.
- Refuse `set_times` with `follow: false` in the run box with `permission_denied`, unless a broker outside the box does it (designed).
  A link's own times take `utimensat` on a name in the link's parent with `AT_SYMLINK_NOFOLLOW`, the built form,
  which the kernel refuses with a `NULL` path, and seccomp can't read the name, so nothing pins that form to the handle:
  allowed in the box, it would retime any file the user owns.
- Change the mode of a file whose mode denies its owner an open, like `0000`, as the Node implementation's path `chmod` does,
  since a path-only open allows any entry (built). The descriptor form `fchmod(fd)` couldn't reach it,
  one more reason that form isn't the built one.
- Return owned buffers from `read`, never a view of memory the runtime reuses (built).
- Keep `/proc/*/environ`, `/proc/*/mem`, and `/proc/*/cmdline` in the read-deny overlay, always on, `-A` included,
  since they're regular files, not magic links (designed; the [deny overlays](glossary.md#deny-overlays)).
- Refuse a read that reaches a denied file through another name (a hard link), by the rule [backstop](backstop.md#open) settles on (designed).

### spawn

- Start a child from one op taking a program resource, argv, and a cwd resource,
  so a call names nothing a path could swap: the pinned program runs by descriptor where the kernel allows it,
  the cwd by a descriptor of its handle's directory, opened beneath its root. A shebang script runs by the path it was found at, since `/proc/self/fd` can't reach one
  and its `$0` is that path, checked against its pin by identity and change time just before the exec (built).
  The interpreter opens it by path after the exec, so the window between that check and the interpreter's open stays open natively too.
- Clear the environment in the op (`env_clear`) and set only what the handle holds,
  with no way for a call to add to it (built).
- Inherit only fds 0, 1, and 2 into the child (built: a run that grants a program marks every descriptor it inherited
  above stderr close-on-exec before anything runs, and what the runtime opens is close-on-exec already).
- Confine the child with the run's kernel rules:
  the project and the linked packages readable, only the declared write roots writable (the tool's own cache directory among them), never the project root,
  TCP on the granted ports, and seccomp denying unix sockets, UDP, new namespaces, and terminal input injection (`TIOCSTI`, `TIOCLINUX`)
  unless a grant names them ([backstop](backstop.md#the-box-for-runs-and-children)) (designed; a child runs unboxed).
- Give a child that needs `/proc` a further Landlock layer of its own before it runs, so the runtime's process falls outside the child's domain
  and its environment stays unreadable (designed).
- Hand the child a `PATH` filtered to absolute entries (built).
- Refuse a name or argument holding a NUL byte or a lone surrogate before it reaches the kernel (built).
- Report a child killed by any signal, real-time ones included, from what `waitid` reports on its pidfd (built).
- Make a group-leading child's process group with `setpgid`, not a new session,
  so a piped child keeps the controlling terminal, and guard it the way `guard_group` does,
  with `guard_children_on_signal` handling the signals that end the host (built).
  A captured child with `stdin: 'ignore'` must fail on `/dev/tty`, not stop on `SIGTTIN`, so it needs a new session or a refused read
  (designed; the built child stays in the terminal's session and stops).
- Leave a captured child with `stdin: 'inherit'` in the host's foreground group, with no `setpgid`,
  since a background group's read of `/dev/tty` stops on `SIGTTIN`, stopping and capping it by pid alone (built).
- Capture `out` and `out_bytes` in the op, enforcing `max_bytes` and reporting `truncated` there,
  decoding only for `out`, whole, so a multi-byte character split across reads stays one character.
  Return `out_bytes`'s streams as owned buffers, never a view of memory the runtime reuses (built).
- Write `stdin` data in the same op, interleaved with the capture so neither pipe can block the other,
  from a buffer copied before the op starts, with `EPIPE` ending the write rather than failing the call
  and no `SIGPIPE` reaching the runtime (built).
- Kill every child still running when the host exits, the group for a group leader, from the runtime's own exit path (built).
- Resolve the `PATH` walk and the realpath pin at grant time, so a handle's reach is decided once,
  skipping `node_modules/.bin` entries by the rule above (built, as the run starts, for each program `--allow-run=` names).
  The policy file names the program, and its pin, a real path that differs per machine, is recorded in a local file that isn't committed (designed).
- Check a name against a narrowed root's set before any resolution, so a name outside it touches nothing on disk (built).
- Decide the lexical check on a name holding `/` natively, refusing an absolute name or one climbing above the project,
  so a patched `String.prototype.split` can't forge a segment (built).
- Resolve `program_in`'s name beneath an exec root's own descriptor: the handle's directory by its real path with no symlink on the way,
  then the name beneath it with `openat2` and `RESOLVE_BENEATH`; launch a native binary by its pin's descriptor,
  `/proc/self/fd/N` through `posix_spawn`, and a script by its found path, checked by identity as above (built).
  Refuse a handle the runtime didn't make, a resource rather than a shape,
  and check that the handle's directory is beneath an exec root of the spawn root's scope before resolving, else `not_granted` (built).
  An exec root may also be a write root, as a build's output directory is ([backstop](backstop.md#the-box-for-runs-and-children)).
- Refuse `env`, `pass_env: 'all'`, and `pass_env` globs on a narrowed root natively,
  so a patched `String.prototype.endsWith` can't pass a glob through (built).
- Read a started child's output without passing a bare chunk through promise resolution:
  each chunk reaches the reader in a null-prototype iterator result (built).
- Expand a `pass_env` glob against the host's environment natively, so script code never enumerates the variables' names (built).
- Check each `pass_env` name against that program's spawn grant, narrowed or not (built).
  Reading a variable and forwarding it are separate grants, so a credential can reach one child without the script reading it.

### The process modules

These hold for the process modules' builtins on `fgro` ([on `fgro`](#the-process-modules-on-fgro)).

- Hold no ambient environment. A grant names each variable, and `get` for a name outside the grants throws `not_granted`,
  `get_all` holding only the granted names that are set.
- Refuse a name holding a NUL byte, a `=`, or a lone surrogate before it reaches the OS.
- Make the environment fixed at startup, with no way for a run to change it.
- Give `exit` the runtime's own path, so the spawn children are killed from it, and validate the code there too.
  A script's exit code passes to the process verbatim, so `1` and `2` are shared with `fgro`'s own failures,
  but for a `0` in a run that was refused an `import()` or a host function's call by rule, which exits 1 ([run](run.md#errors-and-exit-codes)).
- Answer `is_terminal` by `isatty` on the descriptor, and report a broken pipe as `false` from `write` rather than a signal,
  ignoring `SIGPIPE`.
- Return `stdin` chunks as owned buffers, never a view of memory the runtime reuses.

### net

- Resolve a name inside the connector and check the connected address against a written range list
  (loopback, link-local including `169.254.169.254`, CGNAT, ULA, IPv4-mapped IPv6, `0.0.0.0`), never the hostname alone,
  with loopback an explicit grant for dev servers and tests.
- Resolve and connect in the op, reading no JS object for connection options, so nothing on `Object.prototype` reaches the connection.
- Hold only the accepted hosts on the root, a host outside them throwing `not_granted`.
- Follow redirects in the op, each hop's host and connected address checked before anything is sent,
  with this module's header stripping, method rewrite, cap, and refused schemes.
- Apply the Fetch forbidden-header list, so `Host` and SNI can't be steered.
- Ignore ambient `HTTP(S)_PROXY`.
- Show a wildcard host grant like `*.github.com` as the blanket class, since a multi-tenant domain makes it one.

### watch

- Watch the handle's directory by a descriptor opened beneath its root's, opening each subdirectory beneath its parent's descriptor
  (`O_DIRECTORY | O_NOFOLLOW`) and adding its watch through that descriptor, never a path (`IN_DONT_FOLLOW`, `IN_ONLYDIR`),
  and name each event by the watch it came from, so a symlink swapped in is never watched through and nothing is resolved after the scan.
- Keep watching a watched directory that's moved, as the watch's descriptor keeps it, and end on its removal (`IN_DELETE_SELF`).
- Report `IN_Q_OVERFLOW` as a `rescan` of `''`, and a subdirectory whose watch can't be added as a `rescan` of it.
- Scan a new subdirectory once its watch is added, reporting an `add` for each entry found, so a directory filled before its watch lands still reports its entries.
- Read names as bytes, a name that isn't UTF-8 a `rescan` of its directory rather than a lossy path.
