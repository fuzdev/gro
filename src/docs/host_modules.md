# host modules

> the imports that reach the OS, `fs`, `spawn`, `process`, `net`, and `watch`: how to import them, hand them down,
> and what the Node implementation does and doesn't guarantee

> **status**: `fs`, `spawn`, `process`, `net`, and `watch` exist as the Node implementation in Gro's npm package.
> On `fgro` the host modules will be builtins resolved ahead of package lookup
> ([authority](authority.md#imports-or-globals) has the design).

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
for await (const chunk of check.stdout!) process.stdout.write(chunk);
const result = await check.closed;
```

- `spawn.program(name, options?)` makes a `SpawnProgram`, the capability to run one executable,
  resolved once when it's made and pinned for the handle's life.
  `spawn.find(name, options?)` is the same, returning `null` instead of throwing `not_found`,
  and any other failure, like `not_executable`, still throws.
- A program handle bounds which binary runs, not what it does: for most tools the arguments are code
  (`git -c core.sshCommand=…`, `node -e`, `ssh -o ProxyCommand=`, `find -exec`), and a tool reads its own config from the project and `HOME`,
  so on `fgro` a spawn grant is bounded by the kernel backstop around the child, and the policy diff shows it that way ([backstop](backstop.md#the-box-for-runs-and-children)).
- A bare name walks the host's `PATH` synchronously, absolute entries only, to the first regular executable.
  The child's own `PATH` lookups use the host `PATH` as it is, so a relative or empty entry in it resolves against the child's cwd.
  Neither `program` nor `find` looks in `node_modules/.bin`, so a dependency's `.bin/git`
  can't shadow `spawn.program('git')`. Gro's own tasks find a project's tools, like `vite` and `eslint`,
  with `find_cli` (`cli.ts`), which tries the project's `node_modules/.bin/<name>` first and then the bare name,
  while `git`, `node`, and the package manager go straight to `spawn`.
- A name containing `/` resolves against the project root. It's a lexical check:
  a name that's absolute or climbs above the project through `..` is `not_granted`,
  while a symlink beneath the project is pinned wherever it leads, as a `PATH` entry would be.
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
- A result is `{kind: 'exited', code}` or `{kind: 'signaled', signal}`, so a non-zero exit is a value.
  A program that can't start throws `SpawnError` of kind `launch_failed`, as does a script whose found path now leads to another file.
  A captured call that can't allocate memory for a stream kills the child and rejects with `RangeError` once it has ended.
  `not_found`, `not_executable`, `not_granted`, and `launch_failed` are the kinds,
  matched with `is_spawn_error(error, kind?)`. Both live in `@fuzdev/gro/spawn_error.ts`, apart from the host module,
  so matching one grants nothing; on `fgro` it's a builtin that grants nothing, resolved beside `spawn`.
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
  The result is of kind `timed_out` or `aborted`, with the `code` or `signal` the child then ended by,
  the other `null`, so a child that exits 0 on `SIGTERM` isn't mistaken for one that finished,
  and a call never rejects for it. A `signal` already aborted when the call is made starts nothing
  and throws `launch_failed`, its reason on `cause`, since a result would claim a child ran.
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
  Nothing exposes the running children.
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

## process

```ts
import {process} from '@fuzdev/gro/process.ts';

const token = process.env.get('SOME_TOKEN'); // `undefined` when it's unset
const ci = process.env.narrow(['CI']); // a handle limited to `CI`

for await (const chunk of process.stdin) await process.stdout.write(chunk); // bytes
process.set_exit_code(1);
```

The export deliberately shares its name with Node's global, so a module that imports it shadows the global.
A module's use of the process shows in its imports, as for `fs` and `spawn`,
and on `fgro` there's no global at all.

- `process.env` is an `Env`. `get(name)` returns the value or `undefined` when it's unset,
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
- `process.args` are the arguments after the script, a frozen real array, what a script under `gro run` or plain `node` is given.
  In the CLI's own process they're the CLI's arguments, the task name included;
  a task reads its parsed args from its context, since a nested `invoke_task` shares this process.
  With `node -e` the first argument is the first, and a script read from stdin as `-` keeps its arguments too.
- `process.cwd()` is the working directory when the module loaded, checked as for `fs.project`.
- `process.exit(code?)` exits, and without a `code` uses what `set_exit_code` set, else `0`.
  `process.set_exit_code(code)` sets the code the process exits with when it ends on its own.
  A code is an integer from 0 to 255, anything else a `TypeError`, checked with no value of the caller's run,
  so a thenable or a `Symbol.toPrimitive` isn't called.
  `exit` runs the spawn module's child cleanup, like any exit Node runs,
  and it drops writes not yet accepted, so await a write before exiting.
- `process.stdin` is an `AsyncIterable<Uint8Array>`, read once, each chunk a plain `Uint8Array` copy,
  a second iteration a `TypeError`. Leaving a loop early stops reading, so a child inheriting stdin gets what the read hadn't taken,
  and the process can exit with stdin still open.
- `process.stdout` and `process.stderr` take `write(data)`, a string as UTF-8 or a `Uint8Array`,
  resolving once it's accepted, so awaiting each write paces a writer against its reader, and writes not awaited queue in memory.
  A broken pipe, a task piped to `head`, resolves `false` instead of throwing, and every later write resolves `false` too.
  Data that's neither a well-formed string nor bytes is a `TypeError`.
  Any other write failure, like a full disk, rejects with `ProcessError` of kind `io_error`.
- Each stream has `is_terminal`, read from its descriptor without creating the stream.
- `process.pid` is for display, as nothing in the module takes a pid.
  `process.runtime_version` is opaque, like `node/v24.14.0`, for display and for keying a cache by the runtime that made it, compared for equality only.
- Errors are `ProcessError`, typed by `kind` (`not_granted`, `io_error`), its `target` the variable or the stream.
  Match one with `is_process_error(error, kind?)`, in `@fuzdev/gro/process_error.ts`, apart from the host module,
  so matching one grants nothing; on `fgro` it's a builtin that grants nothing, resolved beside `process`.

The module's TSDoc has each op's contract.

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
  as [`env.narrow`](#process) limits names.
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
  so matching one grants nothing; on `fgro` it's a builtin that grants nothing, resolved beside `net`.
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
  The process host module follows the same rule: those roots and the CLI's bootstrap (`gro.ts`)
  import `process`, and library code takes what it needs as parameters, a plain record, a `cwd`
  string, or the CLI args.
  The one exception is `internal/fs_boundary.ts`, which imports `process` to read `HOME` once, when it loads,
  for its protected-directory set, so `create_fs_paths` needs no parameter for a value no root chooses.
  The CLI's entry passes its own to the config's loading and `invoke_task`,
  which carries it in its options to the tasks a task invokes, never on the task's context.
  The rendered `$app/env/public` and `$app/env/private` modules are roots too,
  importing `fs` and `create_fs_paths` to read the `.env` files and `process` for the ambient env
  when they run, so a module importing `$app/env/*` shows those reads in its import graph,
  while the loader that renders them never loads the host module on its hooks thread.
  Gro reads its own `package.json` through the module system (`package_json_load_for_gro`), with no fs grant.
  A function that reads files outside the directory it's named for takes an `FsPaths` rather than a handle,
  like `library_load_from_repo({fs_paths, git}, repo_dir)`,
  since a repo's `package.json`, `tsconfig.json`, or `.gro/` can be a symlink out of it,
  which a handle on the repo would refuse.
  Where a directory's files come from its own listing or search, Gro opens the directory once
  and reads and writes them beneath its handle.
- The boundary's machinery (`internal/fs_boundary.ts`) reaches further than the parameters it's given,
  until the op it needs exists: it reads real paths and symlinks from `node:fs` itself,
  so an `FsPaths` reaches the filesystem beyond the `Fs` it's built from until that read is an op on the root,
  beside `open_dir`, reached through the `Fs` passed in.
- Gro's library and task code otherwise reaches the filesystem only through the fs host module,
  and starts programs only through the spawn host module, so it can run unchanged where the modules are builtins.
  The rest that stays on `node:fs` is Node-only machinery: the loader and the modules it runs in its hooks
  (`resolve_specifier.ts`, `svelte_config.ts`, `sveltekit_shim_app_env_vars.ts`), the Filer family and the Node module graph,
  `spawn.ts`'s `PATH` walk and pin,
  the check of the working directory that `fs.project` and `paths` load from (`internal/cwd.ts`),
  the bootstrapping that finds the Gro install to spawn `node` with (`gro_helpers.ts`),
  and the sync stats behind the task-path hints (`internal/task_hints.ts`),
  which the synchronous args parsing calls, where no sync boundary opens a path from outside.
  `spawn.ts` is the one module on `node:child_process`.
  Gro's code reads the process through its host module, except what stays on the global,
  Node-only like the fs machinery above: `process.on`, `once`, and `kill`, which the module doesn't have,
  so `invoke.ts`'s handlers and `spawn.ts` use them directly, and `spawn.ts`'s live read of the
  environment for a child; the module implementations (`process.ts`, `internal/cwd.ts`);
  and what runs in the loader's hooks or restores a tool's write, `svelte_config.ts`
  (`NODE_ENV`, `MODE`, and the cwd) and `sveltekit_shim_app_env_vars.ts` (the cwd).
  The runtime's builtin has no global, so these move when Gro runs on `fgro`.

- Advisory grants, designed and not built, will count a `node:*` import on the Node Gro as the host module it corresponds to,
  and report one with no entry here as ambient:

  | Node builtin                                                               | host module      |
  | -------------------------------------------------------------------------- | ---------------- |
  | `node:fs`, `node:fs/promises`                                              | fs               |
  | `node:child_process`                                                       | spawn            |
  | `node:process`                                                             | process          |
  | `node:net`, `node:http`, `node:https`, `node:dns`, `node:tls`, `node:dgram` | net              |

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
  and hold their pin, the path they run at, `argv[0]`, and environment in private fields,
  so a method called on any other object throws `TypeError` and changing `path` changes nothing that runs.
- Handles, their prototypes, `spawn`, results, and `SpawnError` with its prototype are frozen.
- A result is a null-prototype record, so a polluted `Object.prototype.then` isn't consulted when it's awaited.
- A `SpawnChild`, its stdin, its output iterables, and their iterators are frozen,
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

### process's invariants

- Handles come only from `process` and `Env.narrow`, through a module-private key, and hold their state in private fields,
  so a method or getter called on any other object throws `TypeError`.
- `process`, its parts, `Env` handles and `get_all`'s records, and their prototypes are frozen,
  and every prototype is null-prototype, so none of them is thenable even with a polluted `Object.prototype.then`.
  `args` is the exception, a frozen real array, so its prototype is the unfrozen `Array.prototype` and awaiting it adopts a polluted `then`.
- An environment read goes through a null-prototype snapshot holding only `process.env`'s own names,
  so an inherited or polluted name reads as unset.
- `narrow` reads its array by index with its length read once, and a name it doesn't allow throws, never `undefined`.
- An exit code is checked by `typeof` first, so nothing the caller passed runs before the refusal.
- A chunk from `stdin` is a plain `Uint8Array` copy, never a `Buffer` or a view of Node's memory.

### net's invariants

- Handles come only from `net` and `narrow`, through a module-private key, and hold their hosts in private fields,
  so `narrow` called on any other object throws `TypeError`. `fetch` is a closure, so a receiver can't reach or replace a handle's hosts.
- `net`, a narrowed handle, its `fetch`, and their prototypes are frozen, and the prototypes are null-prototype.
- One `Request` is built per call, its URL read back from that `Request`, so no caller `toString` runs between the check and the send.
- Every redirect's target is checked before it's sent to, in the one loop `fetch` goes through unless the caller chose `manual` or `error`.
- `narrow` reads `hosts` by own property and by index with its length read once, and a host it doesn't allow throws `not_granted`.
- Every send names its `redirect` and dispatcher, and the request's own fields a polluted key would replace,
  since undici's init dictionary reads a key the init doesn't own through `Object.prototype`, even for a null-prototype init.
  It's no defense against pollution generally, as the [gaps](#net-1) say.

### watch's invariants

- Watchers come only from `watch.dir`, through a module-private key, and hold their state in private fields,
  so a method called on any other object throws `TypeError`.
- `watch`, watchers, their iterators, and their prototypes are frozen, and the prototypes are null-prototype.
- Events and iterator results are frozen null-prototype records, so a polluted `Object.prototype.then` isn't consulted on the way to the reader.
- The handle is read once, for its `path`, and its directory listed through the handle before anything watches it.
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
- `process.on`, `once`, and `kill` have no equivalent in the process host module,
  so the CLI's uncaught-error and output-error handlers and the spawn module stay on the `process` global.

### spawn

- A native binary's pin is a path, resolved and then exec'd by path when called,
  so a file replaced at the pinned path in between runs instead.
  A script's is its identity, so a replacement is refused, but a script rewritten in place keeps its inode and runs as rewritten.
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
- A child killed by a real-time signal is reported as `{kind: 'exited', code: 0}`, since Node passes an empty signal name for one.
- Intrinsics stay pollutable. The module copies at load the Node functions it calls,
  `Array.isArray`, `Number.isSafeInteger`, the `process` functions, the timers, and `Object`'s `assign`, `create`, `freeze`, `keys`, and `hasOwn`,
  and its own objects are null-prototype, but code that patches `Array.prototype`, `Set.prototype`, `String.prototype`,
  `Promise`, `Uint8Array`, `TextDecoder`, `Error`, `EventEmitter.prototype.on`, or the stream classes after the module loads runs inside its ops,
  and a patched `on` or stream method holds the raw `ChildProcess` or stream.
- Node copies a set `NODE_V8_COVERAGE` into every child's environment.
- Piped `stdin` data reaches the child on a socket, as Node's stdio pipes are socketpairs,
  so a child reopening it by path, like `cat /dev/stdin`, fails with `ENXIO`, where a native builtin using a real pipe wouldn't.
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

### process

- The global `process` stays, so any module can read `process.env` or call `process.exit`,
  and a narrowed `Env` bounds only the code that uses it.
- The root `Env` allows every name. The runtime's holds only the granted names, and a name outside them throws `not_granted`.
- The snapshot is taken when the module loads, so a write to `process.env` made before that shows.
- `args` under `node -e` and `node -p` depends on the undocumented `process._eval`, since no documented API says the process has no script.
- `exit` in a Node worker ends the thread rather than the process.
- On POSIX Node's stdio writes to files and terminals block the event loop until the kernel takes them,
  where the runtime's complete on their own schedule. Writes to pipes are asynchronous.
- Node's stream `write` is the stream's own, and the streams are created on first use, so code that patched `write`
  or replaced `process.stdout` by then runs inside a write. The module's other Node functions are copied at load.
  `EventEmitter.prototype` and the stream classes aren't, so code that patches them runs inside these ops holding the raw stream.
- A chunk read from stdin passes through Node's iterator, so a polluted `then` can replace it.
- `process`, like `fs`, reads the working directory when the module loads, so importing it throws
  in a removed or non-UTF-8 working directory.

### net

- The hostname is checked, not the address it connects to, so a granted name resolving to a private range, or rebinding between requests, isn't refused.
- The global `fetch` stays, and any module can import `node:http` or `node:net`, so a handle bounds only the code that uses it.
- The root handle allows every host, where the runtime's holds only the accepted ones.
- Following relies on Node exposing a `manual` response's status and `Location`, where the Fetch standard filters them behind an `opaqueredirect`.
- A dispatcher placed at undici's global key reroutes every send, as for the global `fetch`, and a missing one rejects rather than sends past it.
- Beside native follow, following differs in five ways, each for the reason given:
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

### watch

- The handle is read for its `path`, as spawn reads a `cwd` handle, so a forged handle is any object with an absolute `path` whose `list` resolves.
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

What the runtime's builtin must do that the Node implementation can't:

### fs

- Make each confinement decision in one op, `openat2` with `RESOLVE_BENEATH` beneath the handle's descriptor,
  never a check followed by an open.
- Pass `RESOLVE_NO_MAGICLINKS` explicitly, so a handle on `/proc` refuses its magic links.
- Copy and remove a tree recursively beneath the descriptor, each entry opened relative to its parent's.
- Check `write`'s data is a string or a `Uint8Array`, and mask its `mode` to `0o777`.
- Return owned buffers from `read`, never a view of memory the runtime reuses.
- Keep `/proc/*/environ`, `/proc/*/mem`, and `/proc/*/cmdline` in the always-on read-deny overlay, since they're regular files, not magic links.
- Refuse a read that reaches a denied file through another name (a hard link), by the rule [backstop](backstop.md#open) settles on.

### spawn

- Start a child from one op taking a program resource, argv, and a cwd resource,
  so a call names nothing a path could swap: the pinned program runs by descriptor where the kernel allows it,
  the cwd by its directory handle. A shebang script runs by the path it was found at, since `/proc/self/fd` can't reach one
  and its `$0` is that path, checked against its pin by identity just before the exec.
  The interpreter opens it by path after the exec, so the window between that check and the interpreter's open stays open natively too.
- Clear the environment in the op (`env_clear`) and set only what the handle holds,
  with no way for a call to add to it.
- Inherit only fds 0, 1, and 2 into the child, and confine it with the run's kernel rules:
  the project and the linked packages readable, only the declared write roots writable (the tool's own cache directory among them), never the project root,
  TCP on the granted ports, and seccomp denying unix sockets, UDP, new namespaces, and terminal input injection (`TIOCSTI`, `TIOCLINUX`)
  unless a grant names them ([backstop](backstop.md#the-box-for-runs-and-children)).
- Give a child that needs `/proc` a further Landlock layer of its own before it runs, so the runtime's process falls outside the child's domain
  and its environment stays unreadable.
- Hand the child a `PATH` filtered to absolute entries.
- Refuse a name or argument holding a NUL byte or a lone surrogate before it reaches the kernel.
- Report a child killed by any signal, real-time ones included, from `wait4`'s status.
- Make a group-leading child's process group with `setpgid`, not a new session,
  so a piped child keeps the controlling terminal, and guard it the way `guard_group` does,
  with `kill_guarded_groups_on_signal` handling the signals that end the host.
  A captured child with `stdin: 'ignore'` must fail on `/dev/tty`, not stop on `SIGTTIN`, so it needs a new session or a refused read.
- Leave a captured child with `stdin: 'inherit'` in the host's foreground group, with no `setpgid`,
  since a background group's read of `/dev/tty` stops on `SIGTTIN`, stopping and capping it by pid alone.
- Capture `out` and `out_bytes` in the op, enforcing `max_bytes` and reporting `truncated` there,
  decoding only for `out`, whole, so a multi-byte character split across reads stays one character.
  Return `out_bytes`'s streams as owned buffers, never a view of memory the runtime reuses.
- Write `stdin` data in the same op, interleaved with the capture so neither pipe can block the other,
  from a buffer copied before the op starts, with `EPIPE` ending the write rather than failing the call
  and no `SIGPIPE` reaching the runtime.
- Kill every child still running when the host exits, the group for a group leader, from the runtime's own exit path.
- Resolve the `PATH` walk and the realpath pin at grant time, so a handle's reach is decided once.
- Check a name against a narrowed root's set before any resolution, so a name outside it touches nothing on disk.
- Decide the lexical check on a name holding `/` natively, refusing an absolute name or one climbing above the project,
  so a patched `String.prototype.split` can't forge a segment.
- Refuse `env`, `pass_env: 'all'`, and `pass_env` globs on a narrowed root natively,
  so a patched `String.prototype.endsWith` can't pass a glob through.
- Read stdin without passing a bare chunk through promise resolution.
- Expand a `pass_env` glob against the host's environment natively, so script code never enumerates the variables' names.
- Check each `pass_env` name against that program's spawn grant, narrowed or not.
  Reading a variable and forwarding it are separate grants, so a credential can reach one child without the script reading it.

### process

- Hold no ambient environment. A grant names each variable, and `get` for a name outside the grants throws `not_granted`,
  `get_all` holding only the granted names that are set.
- Refuse a name holding a NUL byte, a `=`, or a lone surrogate before it reaches the OS.
- Make the environment fixed at startup, with no way for a run to change it.
- Own the read of `HOME` behind the protected directories of [Gro's guards](#gros-guards-on-its-own-destructive-paths),
  so `create_fs_paths` grants no environment read to whatever imports it.
- Give `exit` the runtime's own path, so the spawn children are killed from it, and validate the code there too.
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

- Watch the directory the handle holds by descriptor, opening each subdirectory beneath its parent's descriptor
  (`O_DIRECTORY | O_NOFOLLOW`) and adding its watch through that descriptor, never a path (`IN_DONT_FOLLOW`, `IN_ONLYDIR`),
  and name each event by the watch it came from, so a symlink swapped in is never watched through and nothing is resolved after the scan.
- Keep watching a watched directory that's moved, as the handle keeps it, and end on its removal (`IN_DELETE_SELF`).
- Report `IN_Q_OVERFLOW` as a `rescan` of `''`, and a subdirectory whose watch can't be added as a `rescan` of it.
- Scan a new subdirectory once its watch is added, reporting an `add` for each entry found, so a directory filled before its watch lands still reports its entries.
- Read names as bytes, a name that isn't UTF-8 a `rescan` of its directory rather than a lossy path.
