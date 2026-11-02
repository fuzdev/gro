# eval

> `fgro eval <module>` as built today: a module evaluated to JSON data,
> the driver contract, the serializer's rules, the envelope, and the exit codes

> **status**: built; the reference for the current binary
> ([install](runtime.md#install) says how to install a release,
> and the [roadmap](runtime.md#roadmap) what comes later, like determinism modes beyond `deny`).
> Eval loads modules as [run](run.md) does and sees the same globals, apart from what's below.

## Usage

```bash
fgro eval <module>                      # serialize the module's default export
fgro eval <module> --export NAME        # serialize another export
fgro eval <module> --driver - < d.js    # call a driver read from stdin with the module's namespace
fgro eval <module> --jit                # turn V8's JIT on, and with it WebAssembly
fgro eval <module> --timeout 2000       # stop the eval after 2000 ms (default: 10000)
fgro eval <module> --graph              # add the module graph the run loaded to the envelope
```

The module is a `.js`, `.mjs`, `.ts`, or `.mts` file, relative to the working directory or absolute.
`--export` and `--driver` don't combine: a driver is handed the whole namespace.
`--driver` takes only `-`.
Every setting is a flag.
The one environment variable eval keeps is `TZ`, which must name UTC (below), so run it with `TZ=UTC`;
before it starts any thread it clears every other, so nothing it runs reads one ([the box](#the-box)),
though the dynamic loader reads its own first, at exec.
It runs only on Linux, with Landlock ABI 6 or later (Linux 6.12), from a build with its startup snapshot embedded ([the box](#the-box)).

## The engine

- **Jitless** by default: JS runs in V8's interpreter, and `WebAssembly` is absent.
  `--jit` turns the JIT on, and `WebAssembly` is present.
  The box is installed for the same choice: jitless, it refuses new executable mappings ([The box](#the-box)).
- **No code generation from strings**, with or without the JIT:
  `eval`, `new Function`, the async and generator function constructors,
  and a string passed to `setTimeout` throw an `EvalError`.
- **The locale is `en-US`**, whatever the host's `LC_ALL` or `LANG`, so `Intl` formats the same everywhere.
- **The console** writes every level to stderr, uncolored, whatever `FORCE_COLOR` says,
  leaving stdout to the envelope.
- **The time zone is UTC**, or the eval fails as `environment` before any code runs,
  with a message naming `TZ=UTC`.
  UTC's aliases that display as UTC (`Etc/UTC`, `Universal`, `Zulu`, `UCT`) are accepted;
  those that display another name (`Etc/GMT`, `GMT0`, `Greenwich`, which dates name `Greenwich Mean Time`)
  are refused, and so is a zone ICU doesn't know.
- **The heap is capped at 512 MiB**, a provisional cap until it's measured against real configs.
  Near it, the run is stopped and fails as `heap`.
  What a stop can't interrupt (one long builtin call, or allocation under the JIT)
  runs out of memory instead: V8 prints its last garbage collections to stderr,
  and the process writes a prepared `heap` envelope, whose `access` is `null`, and exits 1.
  The cap counts V8's heap only.
  The bytes behind array buffers and typed arrays fall under the process's cap, next;
  a module's source and the envelope fall under the byte caps below;
  and the host's copies of an uncaught error's report (up to 256 errors from causes and an `AggregateError`)
  and of the loader's specifiers are each bounded by the heap's size.
- **The process's memory is capped at 2 GiB**, by its data segment's limit (`RLIMIT_DATA`),
  set soft and hard before any thread starts, so nothing it runs can raise it, and the box refuses the calls that would.
  The cap counts what `malloc` and anonymous mappings take:
  the heap's pages, every thread's stack, and with `--jit` the code range V8 reserves.
  V8's worker threads get an 8 MiB stack, or a smaller default that stands,
  so a large stack limit (`RLIMIT_STACK`) inherited from the caller doesn't spend the cap.
  The cap is about twice what the largest legal runs take (a module at the source cap, a run stopped at the heap's cap, or a value just under the envelope's cap, each with `--jit`), so a config never meets it.
  A lower limit the caller set, soft or hard, stands.
  Past the cap, what happens depends on which allocation fails:
  - an array buffer's: the run is stopped and fails as `memory`,
    or as `heap` when the run holds more than three quarters of the heap's cap at that moment
  - V8's own, on the run's thread: the process writes a prepared `memory` envelope, whose `access` is `null`, and exits 1,
    or a prepared `heap` envelope when V8 reports its heap at fault and the heap is near its cap
  - V8's on one of its own threads (a garbage collection's or a compile's),
    or a Rust allocation (the serializer's output, an error's report):
    the process aborts with no envelope, a message on stderr, and a signal's exit
  - growing a resizable `ArrayBuffer` (`resize`): a `RangeError` the run can catch, and the run goes on

  A caller's limit too low for the engine to start (with `--jit`, one under the code range V8 reserves)
  aborts the process the same way, with no envelope.
  A caller treats a missing envelope as a failure.
- **A module's source is capped at 16 MiB**, as [run](run.md)'s is:
  a larger module is refused as it's read, before any of it is parsed or stripped, and fails as `link`,
  with a message naming the cap and the file.
  The driver, which the caller writes, isn't capped.
- **The envelope is capped at 64 MiB**: the serializer stops as soon as the value's JSON would pass it,
  and an envelope with a value that would pass it whole (long specifiers in `access.dynamic`, say)
  is replaced by a small `serialize` failure naming the cap and the size it would have been,
  whose `access` is `null`, as is its `graph` with `--graph`.
  A failure past the cap keeps its kind: its `access` and `graph` are dropped (`null`),
  and if it's still past, each of its text fields (`message`, `at`, `specifier`, `importer`, `hint`)
  longer than 64 KiB is cut to it, at a character boundary, and named by its length, `aaa… (134217728 bytes)`.
  Neither cap has a flag.
- **The timeout** stops the run after `--timeout` milliseconds, 10000 by default, and fails it as `timeout`.
  It counts from the runtime's start to the drain's end, and stops JS and the serializer's walk;
  a module load in progress finishes first, and so does a builtin call in progress
  (one long `JSON.stringify`, say), bounded by the heap's cap, so a stop can come seconds late.
  Console output goes to stderr, and a write blocks while stderr's reader isn't reading,
  which the timeout can't interrupt: a caller drains stderr while the eval runs.

## Determinism

Eval's one mode is `determinism: deny`: what reads the clock, randomness, or a fact of the machine,
or schedules a timer, throws an `Error` naming the operation and the reason
(`clock`, `random`, `machine`, or `timer`), installed before the driver or any module runs.
Each is replaced where it lives, with its property's attributes kept,
so no global, member, descriptor, prototype, or subclass the run's code can reach leads back to the original.
The web classes' own internals still hold the originals they captured when they loaded,
which no code in the run can read; where one of them would put the clock or randomness in an object's own state,
the class is absent instead (below).

Refused:

- `Math.random()`
- `performance.now()`, `mark()`, `measure()`, `toJSON()`, and `performance.timeOrigin`
- every `Temporal.Now` function
- `Date()`, `new Date()`, and `Date.now()`;
  `new Date(value)`, `Date.UTC`, and `Date.parse` are kept
- `Intl.DateTimeFormat`'s `format()` and `formatToParts()` with no date
- `AbortSignal.timeout()`, and `Atomics.waitAsync()` with a finite timeout
- `Atomics.wait()`, which can't block the main thread (a `TypeError`)

Absent: `setTimeout`, `setInterval`, `clearTimeout`, and `clearInterval`.
With no timers, work left after the evaluation runs as promise jobs, which all run before the value is taken.
Absent too: `Blob`, `File`, `URL.createObjectURL()`, and `URL.revokeObjectURL()`.
A `File` keeps the time it was made in a field of its own, which `structuredClone` copies, and every blob part has a random id.

Inert: `WeakRef` holds its target, so `deref()` always returns it;
`FinalizationRegistry` never calls its callback;
`console.time()`, `timeLog()`, and `timeEnd()` print nothing.

## The protocol

1. The module resolves; if it doesn't, the kind is `resolve`.
   It must be a project's own module, so its project root bounds what the eval reads:
   a module inside `node_modules`, or one with no project root above it
   (no `package.json` outside `node_modules` that holds more than `"type"` and `"sideEffects"`,
   up to the filesystem's root or a world-writable directory), is refused as a `resolve` too, exiting 2.
2. With `--driver -`, the driver evaluates first, before any of the module's code,
   so it holds the realm's original `JSON`, `Object`, and prototypes before the module could patch them.
3. The module's graph loads and evaluates.
4. With a driver, its `default` export is called with the module's namespace and awaited,
   and what it returns is the value.
   With no driver, the value is the namespace's export, `default` unless `--export` names another.
5. The value is serialized (below).
6. The event loop drains: work still pending runs, and an uncaught error or unhandled rejection in it
   fails the eval with no value (`late_rejection`).
7. With a driver that exports `after_drain`, it's called, and a throw is a `late_rejection` too.

Every `import()` in the run is refused before anything is resolved, whatever it names,
with a `TypeError` the code can catch; each `import()` in the module's graph is listed in the envelope's `access.dynamic`,
caught or not (the driver's are refused as imports from the driver, and not listed). `import.meta.resolve` works as in [run](run.md).

Each `package.json` the run's resolutions read is read once:
a later resolution that finds the same file parses the first read's bytes,
even when the file was rewritten in place since,
and one that finds another file at its path (a file renamed over it after eval first read it) fails to resolve.

## The box

Eval's process is held in a kernel box before any of the module's code, or V8 itself, runs.
The box's roots are the project root and each package linked into the project's `node_modules` from outside it,
found from the directory structure alone before the box goes up:
every entry of the project's `node_modules` (and of each `@scope` directory in it) whose real path is a directory outside the roots so far,
with a `package.json` file of its own (not a symlink), becomes a root,
and its own `node_modules` is read the same way, until nothing new is found.
A package installed in the project's `node_modules` is under the project root already.
A link to a file, to a directory with no `package.json` file, to a directory holding a root (an ancestor of the project, or `/`),
or to one a world-writable directory holds, which the resolver refuses to read a package through, adds nothing.
Each root is opened as it's found, and the box goes up on the directories opened,
so one swapped in at a root's path afterward isn't what the box admits.

Before anything else, the process closes every file descriptor above stderr it inherited:
a file, a socket, or a device its parent left open, which the box, gating only new opens, wouldn't reach.
If they can't be closed, the eval fails as `environment`.
Then it clears its environment but `TZ`, from its one thread, so nothing it runs after,
deno_core, V8, ICU, or glibc, reads another variable, and `RUST_BACKTRACE` has no effect on a panic; if it can't, the eval fails as `environment`.
Then it caps its memory ([the engine](#the-engine)), before any thread starts, so the cap holds for every thread; if it can't, the eval fails as `environment`.
The dynamic loader reads its own variables (`LD_PRELOAD`, `LD_LIBRARY_PATH`, glibc's tunables) before that, at exec.
The process resolves the module and finds the roots on a thread it then ends,
installs the box from its one remaining thread, and only then starts V8 on another,
so every thread V8 and its event loop start is inside the box.
The process refuses to install the box with any other thread alive, and fails as `internal`.
The walls, in order:

- **`no_new_privs`**: nothing the process could run gains privileges.
- **Landlock**: only files and directories under the roots can be read:
  no write anywhere, no execution, nothing outside the roots (`/proc` and `/dev` included);
  no TCP socket can bind or connect;
  and no signal reaches a process outside the box, nor can it connect to an abstract unix socket outside it.
  Every rule is enforced in full, or eval doesn't run.
  Landlock doesn't gate a path's metadata: `stat`, a symlink's target, and so whether a path exists stay readable everywhere.
- **seccomp**, two filters. The first kills the process on `execve` and `execveat`, `ptrace`, the cross-process memory calls,
  `bpf`, `perf_event_open`, `userfaultfd`, the `kexec` calls, the `io_uring` calls, and the kernel module calls,
  since nothing in the runtime makes them: a tripwire for code that escaped the engine.
  The second is an allowlist: the calls the runtime makes once the box is up pass,
  and every other call fails with `ENOSYS`.
  The allowed calls (the ones the census saw, then their siblings, each sibling with its reason in the source) cover memory (`madvise`, `mremap`, `munmap`, and the rest),
  threads and their futexes, the clock, sleeping, signal handlers and `abort`, `read`, `write`, `openat`, `close`, `lseek`,
  `statx` (and `fstat` and `newfstatat`, std's fallback), `readlink`, `uname`,
  the event loop's `epoll` and `eventfd2` calls, `getrandom`, the process's and thread's ids,
  and reading scheduling and CPU affinity (another process's included, which tells only whether it exists and how it's scheduled).
  Some pass only with the arguments the runtime uses:
  `prctl` to name a thread or a mapping;
  `fcntl` to read and set a descriptor's flags and duplicate it close-on-exec (no locks, leases, or change notices);
  `ioctl` for non-blocking and close-on-exec, the bytes waiting, and a terminal's settings and size;
  `socketpair` as a unix stream pair only
  (a datagram pair could send to a socket file outside the box, which Landlock below ABI 9 doesn't gate);
  `faccessat2` only to ask whether a path exists (`F_OK`, as glibc's `realpath` does), which `statx` tells already;
  and `clone` only to start a thread, with no namespace flag, so no process is forked and no namespace made.
  When the JIT is off, no mapping can be made executable:
  `mmap`, `mprotect`, and `pkey_mprotect` with `PROT_EXEC` fail with `ENOSYS`,
  and with `--jit` they pass whatever they ask, since the JIT's compiled code needs executable memory.
  What was mapped before the box went up, the binary's code and its libraries', stays executable,
  and `mremap` can move, copy, or grow it, over the same files' bytes.
  `clone3` isn't allowed, since its flags are in memory, out of a filter's reach;
  glibc falls back to `clone` when it fails.
  So every `socket`, `connect`, `bind`, and `listen`, every call that changes a file's metadata
  (which Landlock at ABI 6 doesn't gate), `kill`, file watches, System V IPC and POSIX message queues,
  resource limits (`getrlimit`, `setrlimit`, and `prlimit64`, so the memory cap set before the box stays) and priorities,
  namespaces, keys, and further seccomp filters fail with `ENOSYS`.
  `ENOSYS` is what a call the kernel lacks returns, so glibc and Rust's std fall back where they can,
  and a host whose libc makes a call the list doesn't name degrades the same way or fails with `ENOSYS`.
  An x32 call fails too, since the filter compares exact call numbers.
  The list is built for x86_64 and aarch64, and has been checked on x86_64 only.

The reads the runtime makes at startup outside the roots (the CPU and cgroup files, `/proc/self/maps`) fail and fall back quietly.
V8's entropy comes from the `getrandom` call, and if that fails, the process stops with a message
rather than let V8 seed itself from the clock.

Eval refuses to run, as `environment`, on a kernel without Landlock, with it switched off,
or with it below ABI 6, which the signal and abstract unix socket walls need, naming the kernel's ABI;
on an architecture the seccomp filters aren't built for (they are for x86_64 and aarch64)
or a kernel that refuses them; and on a platform other than Linux.
A build with no embedded startup snapshot is refused as `internal`:
started from source, the runtime would read its extensions' JS from the build tree, outside the box.
There's no flag to run with a weaker box or none.

On top of the box, every module and `package.json` read is checked against the roots before it happens,
a `package.json` served from the run's earlier read of it included.
A module graph can reach past them: the resolver follows links the closure doesn't
(one in a nested `node_modules`, or in a subdirectory's own `node_modules`)
or drops (one to a directory holding the project or another package's directory, which `fgro run` follows),
and reads a `package.json` symlinked out of the roots at its real path.
Such a read is refused, and the eval fails as `internal`, naming the path, whatever the code did with the error;
the fix is to link the package's own directory directly into the project's `node_modules`, and to keep each `package.json` a file.

### Changing the allowlist

The allowlist comes from a census: what the runtime calls once the box is up, on a real kernel.
To take one, in a scratch copy of the repo:

1. Change the allowlist's mismatch action in `seccomp_filters` to `SeccompAction::Log` (`SECCOMP_RET_LOG`), allowing the candidate list, and build with `cargo xtask build`.
2. Run the eval tests' fixtures and real projects' config modules through `fgro eval`, with and without `--jit` and `--graph`, and with a driver.
3. Read the kernel's audit records, `type=1326 ... syscall=N`, with `journalctl -k`.
   The printk rate limit drops records, shown as `kauditd_printk_skb: N callbacks suppressed`,
   so pause between runs.
4. Add each call logged, and repeat until a run logs nothing and nothing is suppressed.

Then restore `ENOSYS`, run `cargo xtask build --verify`, and check those config modules exit as before.
Take it again when deno_core, tokio, V8, the Rust toolchain, or the host's glibc or kernel moves
(glibc 2.41 on Linux 6.11 or later takes `getrandom` from the vDSO, so what a census sees depends on both).
A call the census didn't see is added only as a sibling, with a reason the code states.

## The driver

A driver is an ES module read from stdin, for a tool that drives the module (a builder API, say)
rather than reading an export.
It runs in the same realm, first, with the URL `gro:driver`, and it's import-free:
a static import, `import()`, or `import.meta.resolve` in it fails with "a stdin driver is import-free".
Its exports:

- `default`, a function, required: called with the module's namespace;
  its result, awaited, is the value
- `after_drain`, a function, optional: called after the drain,
  so a driver can fail an eval whose module called into it too late, even when the module caught the driver's error

## The serializer

The host walks the value over V8's own state, never the realm's `JSON` or `Object`,
which the module can patch, and runs no JS: no getter, no proxy trap, no `toJSON`.

Written:

- `null`, booleans, and strings, escaped as `JSON.stringify` escapes them
- finite numbers as JS writes them, with `-0` as `0`
- arrays whose prototype is `Array.prototype`, every index from 0 to the length;
  an array's other properties are left out, as `JSON.stringify` leaves them
- plain objects, whose prototype is `Object.prototype` or null:
  own enumerable string keys, integer-like keys first in ascending order, then the rest in insertion order;
  a member holding `undefined` is left out, and non-enumerable members are skipped
- a value reached twice by different paths, written twice
- an object of a kind V8 doesn't name (a `WeakRef`, an array or string iterator, an `Intl` formatter, a web API object)
  whose prototype was set to `Object.prototype` or null is written as a plain object:
  its own enumerable data properties, never its internal state

Refused, with the JS path to it (`$` the value, then `.name`, `["a key"]`, `[2]`;
a key longer than 64 UTF-16 units, an identifier too, is cut to them and named by its length, `["aaa…" (70000 units)]`):

- a getter or setter, never run: on an enumerable member, or at an array index
- `undefined` as the value or in an array, and a hole in an array
- a function, a bigint, a symbol, `NaN`, `Infinity`, `-Infinity`
- an object with any other prototype (a class instance), and an array with another prototype
- a `Map`, `Set`, `Date`, `RegExp`, `Promise`, error, boxed primitive, typed array, buffer, `arguments`,
  module namespace, or `Proxy`, or a host object, whatever its prototype
- a value that one of its members leads back to (a cycle)
- a lone surrogate in a string or key, which a UTF-8 reader can't decode
  (`JSON.stringify` writes one as a `\udXXX` escape most JSON parsers refuse)
- nesting deeper than 1000 arrays and objects
- a value whose JSON would take the envelope past its cap of 64 MiB ([the engine](#the-engine))

## The envelope

Eval writes one JSON object to stdout, on one line, once it has started.
A usage error prints to stderr with no envelope.

```json
{
	"eval_protocol": 2,
	"ok": true,
	"runtime": {
		"fgro_version": "0.0.0",
		"commit": "…",
		"dirty": false,
		"v8": "…",
		"deno_core": "…",
		"tsv_rev": "…"
	},
	"entry": {"id": "/abs/path/config.ts", "export": "default", "driver_blake3": null},
	"value": {},
	"access": {"dynamic": []}
}
```

- `eval_protocol` is `2`; it changes with any change to the envelope a reader must know about, such as a new field or a new error kind,
  and a reader refuses a protocol it doesn't know.
- `runtime` is for display: two binaries can report the same fields,
  so a caller that pins the binary hashes the file itself.
  `commit` and `dirty` are null for a build that embedded no commit.
- `entry.id` is the module's canonical path, or the path as given when it didn't resolve.
  `export` is null with a driver, and `driver_blake3` is the BLAKE3 of the driver's bytes, in hex, or null.
- `value` is present when `ok` is true; `error` takes its place when it's false:
  `{kind, message, at?, specifier?, importer?, hint?}`.
  `at` is the JS path the serializer refused, or the `url:line:column` of a load failure;
  `specifier` and `importer` name the import that failed to resolve or load.
  `hint` is a fix, apart from the message, which stays the error's own report:
  `--jit` for `WebAssembly` not defined in a jitless eval and for a jitless `timeout`,
  that code generation from strings is off for an uncaught `EvalError`
  (V8 throws one natively for nothing else),
  how to link a package for a read outside the box, and the build command for a build with no snapshot.
- `access.dynamic` lists each refused `import()` in the module's graph as `{specifier, importer}`,
  in specifier order, then importer order, one entry per specifier and importer.
  `access` is `null` only in the out-of-memory envelopes, prepared before the run,
  and in an envelope past its cap ([the engine](#the-engine)).
- `graph`, last, is present only with `--graph` ([the graph](#the-graph)).
  Without the flag, the envelope is exactly as above.

The kinds, the contract a caller branches on, derived from the phase that failed and the loader's record,
never from a message;
a failure past the cap keeps its kind, its `access` and `graph` dropped and its long text fields cut:

| kind             | when                                                                                                                   |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `resolve`        | the module didn't resolve, or isn't a project's own module (inside `node_modules`, or with no project root), or a static import didn't resolve (an import in the driver among them) |
| `link`           | the graph didn't form: a syntax error, a missing export binding, TypeScript the strip refuses, import attributes, or a module over 16 MiB |
| `module_threw`   | a module's evaluation, or the driver's call, threw, rejected, or left a top-level `await` unsettled                   |
| `export_missing` | the export to serialize is absent, or the driver has no `default` function (or an `after_drain` that isn't one)       |
| `serialize`      | the value isn't data by the rules above, or the envelope holding it would pass 64 MiB                                  |
| `late_rejection` | the drain threw or rejected, or `after_drain` threw                                                                     |
| `heap`           | the heap reached its cap: the run was stopped, or the process ran out of memory                                       |
| `memory`         | the process reached its memory cap outside the heap's cap: an array buffer's allocation failed and the run was stopped, or V8 ran out of memory with the heap not at fault |
| `timeout`        | the run passed its timeout and was stopped                                                                             |
| `environment`    | the process's time zone isn't UTC, the descriptors it inherited can't be closed, its environment cleared, or its memory capped, or the host can't hold the box: no Landlock, Landlock below ABI 6, an architecture or kernel the seccomp filters don't hold on, or a platform other than Linux |
| `internal`       | the host failed: a module changed on disk between resolving and loading, a read failed, a read outside the box's roots was refused, the build has no snapshot, the graph names a `package.json` with no read of it kept, or a host invariant broke |

## The graph

With `--graph`, the envelope ends with the module graph the run loaded:
each module it resolved, each `package.json` a resolution read, and each path a resolution went past finding nothing there.

```json
"graph": {
	"graph_version": 1,
	"roots": ["project:src/config.ts"],
	"modules": [
		{
			"id": "project:src/config.ts",
			"blake3": "…",
			"origin": {"kind": "project"},
			"format": "typescript",
			"imports": [{"specifier": "@x/lib", "resolved": "linked:../lib/index.js", "reach": "static"}]
		}
	],
	"consulted": [{"id": "project:package.json", "blake3": "…", "role": "search_bound"}],
	"absent": ["project:src/node_modules/@x/lib", "project:src/package.json"],
	"local": {
		"project": "/abs/project",
		"box": ["/abs/project", "/abs/lib"],
		"paths": {"project:src/config.ts": "/abs/project/src/config.ts"}
	}
}
```

A file's id is its real path relative to the project's real directory, P (the [project root](#the-protocol)),
so an id doesn't depend on where the project lives (a module's value can, through `import.meta.url`):
`project:<path>` under P, `node_modules` included,
and `linked:<path>` outside it, with `..` segments (`linked:../lib/index.js` for a package linked from beside the project).
The path is joined by `/`, with `%`, `?`, and `#` percent-encoded,
and a module's query and fragment join its id as its URL has them (`project:src/a.js?x#y`),
so an import with a query or fragment is a module of its own, as it is to V8.
The package's name and version are only in `origin`.
No module is built in yet, so `graph_version` 1 has no `builtin:` id and no `builtin` origin.
Absolute paths are only in `local`.
Every array is sorted, so the dump is a function of what was read, not of the order the run reached it in.

| field                | what                                                                                                                           |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `graph_version`      | `1`; it changes with any change to the dump a reader must know about, such as a new field, id prefix, origin, format, reach, or role |
| `roots`              | the module the graph loads from: the entry alone (the driver, which has no file, is named by `entry.driver_blake3`)           |
| `modules`            | each module the run resolved, by id                                                                                            |
| `modules[].blake3`   | the BLAKE3 of the bytes read for it, before any type strip, in hex; `null` if it didn't load, which only a failed run leaves |
| `modules[].origin`   | `{"kind": "project"}`, or `{"kind": "package", name, version, linked}`: the name it was first imported by, its `package.json` `"version"` (`null` when that isn't a string), and whether its real root is outside every `node_modules` |
| `modules[].format`   | `javascript`, run as read, or `typescript`, stripped of its types; `null` if it didn't load                                     |
| `modules[].imports`  | each import of it that resolved, `{specifier, resolved, reach}`, by specifier then reach: the specifier as written, the id of the module it resolved to, and `static` or `dynamic` (eval refuses every `import()` before resolving it, so none is `dynamic` yet) |
| `consulted`          | each `package.json` a resolution read, `{id, blake3, role}`, by id then role, once per role: `exports` (a package's, for its `"exports"`), `package_scope` (the nearest above a module, for its `"type"`, or above an importer, for a self-reference), `search_bound` (one on the walk up for the project root), `version` (the package's an entry inside `node_modules` is in, which eval refuses, so none appears yet) |
| `absent`             | each path a resolution went on past finding nothing of the kind it looked for, by id: a `package.json` on a walk up, with no file there, and a package in a `node_modules` the search passed, with no directory there; each is the path as looked up |
| `local.project`      | P's absolute path                                                                                                              |
| `local.box`          | the [box](#the-box)'s roots, P first, then the link closure                                                                     |
| `local.paths`        | each file's absolute path, keyed by its id; a module's is under its id cut at the first `?` or `#`                              |

Each `package.json` is read once per run ([the protocol](#the-protocol)),
so a consulted file's `blake3` is the hash of the bytes every resolution in the run parsed.

What the run depended on is `modules` and `consulted` by id and hash and `absent` by id:
a file edited, added in a directory a walk or the package search passed, or moved under a link
changes one of them.
A cache keyed on them, with the driver's hash, the runtime, and the invocation
(`entry.export`, and `--jit`, which brings WebAssembly),
never keys on `origin`, whose name is whichever import reached the package first.
Some inputs aren't recorded:
where the project lives, which a module can read as `import.meta.url`,
so a cache keys on the entry's canonical path (`entry.id`) too;
what `import.meta.resolve` resolved to, whose file is a module of the graph only if it's also imported,
and when it fails and the code catches it, the `package.json` files it read;
a directory's mode: one a walk passed or a package was found through turning world-writable makes a fresh run refuse;
and the box's roots: a link added to `node_modules` that changes the closure can make a fresh run fail as `internal`.

`graph` is `null` when the run never reached its module graph
(the module didn't resolve, the box didn't go up, or the process ran out of memory and wrote its prepared envelope),
or when a path in it isn't UTF-8, which no id can name, or a module's URL names no file,
or when the envelope would pass its cap, which drops it (and fails an `ok: true` one as `serialize`);
those are the only cases, and under `ok: true` the value stands but there's no graph to cache it on.
A graph the host failed to record (a `package.json` consulted with no read of it kept) fails the eval as `internal`.
A failed run still dumps the graph as far as it got, for diagnosis, under `ok: false`:
a module that resolved but never loaded has a `null` `blake3`,
and the `package.json` files a failed resolution read aren't in `consulted` (the paths it went past are in `absent`).
So only an envelope with `ok: true` describes a run whose graph a caller may cache on.
A reader refuses a `graph_version` it doesn't know.

## Exit codes

The same table as [run](run.md#errors-and-exit-codes):

| exit | when                                                                                                     |
| ---- | -------------------------------------------------------------------------------------------------------- |
| 0    | the value serialized, or `--help`                                                                        |
| 1    | any failure after the module resolved, the kind in the envelope                                          |
| 2    | a usage error (an unknown flag, `--driver` other than `-`, `--driver` with `--export`, a `--timeout` below 1), or a module that doesn't resolve or isn't a project's own |

A module that fails to resolve on an I/O error or a symlink loop exits 1, not 2, as in `run`.
A failure writing the envelope (a closed pipe, a full disk) is reported on stderr and exits 1, whatever the eval's outcome.
