# eval

> `fgro eval <module>` as built today: a module evaluated to JSON data,
> the driver contract, the serializer's rules, the envelope, and the exit codes

> **status**: built in part; the reference for the current binary, which has no release yet.
> Eval loads modules as [run](run.md) does and sees the same globals, apart from what's below.
> Built today are the protocol, the serializer, the engine's settings, `determinism: deny`,
> and the heap and time limits.
> Not yet built is the kernel backstop ([runtime](runtime.md#roadmap)).

## Usage

```bash
fgro eval <module>                      # serialize the module's default export
fgro eval <module> --export NAME        # serialize another export
fgro eval <module> --driver - < d.js    # call a driver read from stdin with the module's namespace
fgro eval <module> --jit                # turn V8's JIT on, and with it WebAssembly
fgro eval <module> --timeout 2000       # stop the eval after 2000 ms (default: 10000)
```

The module is a `.js`, `.mjs`, `.ts`, or `.mts` file, relative to the working directory or absolute.
`--export` and `--driver` don't combine: a driver is handed the whole namespace.
`--driver` takes only `-`.
Eval reads no environment variable for its own configuration: every setting is a flag.
It runs only with the time zone UTC (below), which ICU takes from the host's `TZ`, so run it with `TZ=UTC`.

## The engine

- **Jitless** by default: JS runs in V8's interpreter, and `WebAssembly` is absent.
  `--jit` turns the JIT on, and `WebAssembly` is present.
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
  runs out of memory instead: the process writes a prepared `heap` envelope, whose `access` is `null`, and exits 1.
  V8 prints its last garbage collections to stderr first.
  The cap counts V8's heap only. Outside it, uncapped, are the bytes behind an `ArrayBuffer`, `SharedArrayBuffer`, or typed array,
  a module's source and its parse, and the envelope's own bytes (serializing a large string makes several copies of it),
  so a runaway allocation there can end the process with no envelope.
  A caller treats a missing envelope as a failure, and a caller that wants a hard bound on memory sets one on the process.
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

## What eval reads

Eval reads modules and `package.json` files only under its read roots:
the project root, and each package linked into the project's `node_modules` from outside it.
Those are found before the module runs, from the directory structure alone:
every entry of the project's `node_modules` (and of each `@scope` directory in it) whose real path is a directory outside the roots so far,
with a `package.json` file of its own (not a symlink), becomes a root,
and its own `node_modules` is read the same way, until nothing new is found.
A package installed in the project's `node_modules` is under the project root already.
A link to a file, to a directory with no `package.json` file, to a directory holding a root (an ancestor of the project, or `/`),
or to one a world-writable directory holds, which the resolver refuses to read a package through, adds nothing.

Every read is checked against the roots before it happens.
A module graph can reach past them: the resolver follows links the closure doesn't
(one in a nested `node_modules`, or in a subdirectory's own `node_modules`)
or drops (one to a directory holding the project or another package's directory, which `fgro run` follows),
and reads a `package.json` symlinked out of the roots at its real path.
Such a read is refused, and the eval fails as `internal`, naming the path, whatever the code did with the error;
the fix is to link the package's own directory directly into the project's `node_modules`, and to keep each `package.json` a file.

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
- an object of a kind V8 doesn't name (a `WeakRef`, an iterator, an `Intl` formatter, a web API object)
  whose prototype was set to `Object.prototype` or null is written as a plain object:
  its own enumerable data properties, never its internal state

Refused, with the JS path to it (`$` the value, then `.name`, `["a key"]`, `[2]`):

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

## The envelope

Eval writes one JSON object to stdout, on one line, once it has started.
A usage error prints to stderr with no envelope.

```json
{
	"eval_protocol": 1,
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
  and that code generation from strings is off for an uncaught `EvalError`
  (V8 throws one natively for nothing else).
- `access.dynamic` lists each refused `import()` in the module's graph as `{specifier, importer}`,
  in specifier order, then importer order, one entry per specifier and importer.
  `access` is `null` only in the out-of-memory envelope, prepared before the run.

The kinds, the contract a caller branches on, derived from the phase that failed and the loader's record,
never from a message:

| kind             | when                                                                                                                   |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `resolve`        | the module didn't resolve, or isn't a project's own module (inside `node_modules`, or with no project root), or a static import didn't resolve (an import in the driver among them) |
| `link`           | the graph didn't form: a syntax error, a missing export binding, TypeScript the strip refuses, or import attributes |
| `module_threw`   | a module's evaluation, or the driver's call, threw, rejected, or left a top-level `await` unsettled                   |
| `export_missing` | the export to serialize is absent, or the driver has no `default` function (or an `after_drain` that isn't one)       |
| `serialize`      | the value isn't data by the rules above                                                                                |
| `late_rejection` | the drain threw or rejected, or `after_drain` threw                                                                     |
| `heap`           | the heap reached its cap: the run was stopped, or the process ran out of memory                                       |
| `timeout`        | the run passed its timeout and was stopped                                                                             |
| `environment`    | the process's time zone isn't UTC                                                                                      |
| `internal`       | the host failed: a module changed on disk between resolving and loading, a read failed, a read outside the read roots was refused, or a host invariant broke |

## Exit codes

The same table as [run](run.md#errors-and-exit-codes):

| exit | when                                                                                                     |
| ---- | -------------------------------------------------------------------------------------------------------- |
| 0    | the value serialized, or `--help`                                                                        |
| 1    | any failure after the module resolved, the kind in the envelope                                          |
| 2    | a usage error (an unknown flag, `--driver` other than `-`, `--driver` with `--export`, a `--timeout` below 1), or a module that doesn't resolve or isn't a project's own |

A module that fails to resolve on an I/O error or a symlink loop exits 1, not 2, as in `run`.
A failure writing the envelope (a closed pipe, a full disk) is reported on stderr and exits 1, whatever the eval's outcome.
