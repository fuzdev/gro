# run

> `fgro run <file>` as built today: what it loads, what the global scope holds,
> how a run fails, and the exit codes

> **status**: built; the reference for the current binary, which has no release yet.
> The README's [Develop](../../README.md#develop) section says how to build it,
> and [runtime](runtime.md) has the design it grows into.

## Usage

```bash
fgro run <file>   # a .js, .mjs, .ts, or .mts module, relative to the working directory or absolute
fgro --version    # the version, the git commit, the V8 version, and whether a startup snapshot is embedded
```

`--version` prints one line, `fgro <version> (commit <sha>, V8 <version>, snapshot)`.
The commit is the full SHA the binary was built from, with `-dirty` appended when the working tree differed from it,
untracked files included and the bench's history file excepted;
`cargo xtask build` embeds it, and a binary from a plain `cargo build` embeds none (`commit unknown`).
The last part is `no snapshot` for a build without a startup snapshot.

`run` takes only the entry file, so an extra argument is a usage error and there are no script arguments.
User code can't read stdin or the environment (there is no `process`, `Deno`, or `Bun`).
The event loop drains before the run ends, so timers and pending promises complete,
and a top-level `await` on a promise that never settles ends the run as an error, not a hang.
Linux x86_64 only.
A binary from `cargo xtask build` starts from its embedded snapshot and runs anywhere,
while a plain `cargo build` runs only from its build tree.

## What loads

- `.mjs`, `.ts`, and `.mts` modules, and `.js` modules under a `package.json` with `"type": "module"`.
  A `.js` file without one is refused, never sniffed.
  A world-writable directory (other-write, like `/tmp`) ends the search for it, and for every `package.json`:
  no `package.json` or `node_modules` in it is read,
  and a package installed through one (its `node_modules`, `@scope`, root, a linked root's real holders, or a linked `node_modules`'s or `@scope`'s real parent) is refused.
  TypeScript is stripped of its types in process ([typescript](typescript.md)).
- Relative and absolute specifiers with their full extension,
  and bare specifiers through a package's `"exports"` in `node_modules`,
  searched from the importer up to its project root ([resolver](resolver.md)).
  Every module stays inside its root, checked on its real path:
  a relative or absolute specifier resolves only inside the importer's
  (the entry's project root, or a package's real directory),
  never into `node_modules` below it or into a package admitted inside it,
  and a package's `"exports"` targets and self-references stay inside the package.
  Each refusal names the path, the root, and a fix,
  and so does a file reached both by a relative path and by its package's name, in either order.
  An entry inside `node_modules` runs as the package it's in.
  A module with no project root above it (no `package.json` outside `node_modules` that isn't a type marker,
  below any world-writable directory) gets no bare imports except a self-reference,
  which resolves through its own package's `"exports"` and stays inside its package,
  and any other bare import is refused with the fix: a `package.json` at the project root,
  with the package installed in the `node_modules` beside it.
  A module's `import.meta.url` is its real path, after symlinks.
- A `./x.js` specifier whose file is missing while `./x.ts` exists is refused with a hint naming the sibling,
  as is `.mjs` beside `.mts`, and an extensionless specifier gets a hint naming the file it likely meant.
- Refused with the reason are JSON modules and other import attributes,
  any other extension (`.cjs`, `.json`, `.wasm`), `.cts`, `.tsx`, declaration files, Svelte modules,
  `node:`, `npm:`, `jsr:`, `https:`, every other scheme but `file:`, and `#` imports (the last comes later).
  A Node built-in's bare name, like `fs`, fails as a package that isn't installed.

## The global scope

The ECMAScript globals, including `Intl`, `Temporal`, and `WebAssembly`, plus these web globals:

- `console`, `queueMicrotask`, `structuredClone`, `performance`
- `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`
- `URL`, `URLSearchParams`, `URLPattern`
- `TextEncoder`, `TextDecoder`, `TextEncoderStream`, `TextDecoderStream`, `atob`, `btoa`
- `ReadableStream`, `WritableStream`, `TransformStream`, their readers, writers, controllers, and queuing strategies
- `Blob`, `File`, `CompressionStream`, `DecompressionStream`
- `AbortController`, `AbortSignal`, `Event`, `EventTarget`, `CustomEvent`, `ErrorEvent`, `PromiseRejectionEvent`, `DOMException`

Absent today, by design until a grant or a consumer brings them, are `fetch`, `Headers`, `Request`, `Response`,
`WebSocket`, `crypto`, `self`, `navigator`, `location`, `reportError`, `Worker`, `MessageChannel`,
`MessagePort`, `BroadcastChannel`, `FileReader`, and web storage.
The global object isn't an `EventTarget`, so there's no `addEventListener('unhandledrejection')`.

`import.meta` has `url`, `main`, `resolve` (one argument), `filename`, and `dirname`.
`import.meta.resolve` runs the same strict resolver as an import,
so unlike the web's it throws a `TypeError` for a file that's missing or refused.
It admits no package and records nothing, so the URL it returns for a package is only a path.
For an installed package, `import(import.meta.resolve('foo'))` is refused as a path into `node_modules`,
with the fix to import `foo` by name.
A linked package's URL is a path like any other:
an in-tree one loads as the project's file, since its package was never admitted around it,
and an out-of-tree one is outside the project's root, refused.

## Errors and exit codes

Errors print to stderr prefixed `error:`.
An uncaught error prints `Uncaught Error: message`, read `Uncaught (in promise)` for a module's top-level throw
or an unhandled rejection, then V8's frames as `at file:///path:line:column`,
then each `cause` as `Caused by:` with its frames.
TypeScript positions are the source's, with no source map.
A module that fails to resolve, load, or strip prints its reason without `Uncaught`
(`error: cannot resolve '…' from …`), and a refused TypeScript construct names itself, a fix, and its position.
A dynamic `import()` that fails rejects with a `TypeError` the code can catch.
A module whose file was replaced between resolving and loading isn't loaded
(`cannot load <url>: the file at its path is not the one resolved`;
[resolver](resolver.md#departures-from-node) has what it catches and what it can't),
the entry's included, which exits 1.

`console.log` and `console.info` write to stdout, `console.warn` and `console.error` to stderr.
Whether a stream is colored is decided once per process, in order:

- `FORCE_COLOR` set, even empty: colored for `''`, `1`, `2`, `3`, or `true`, as in Node, and uncolored for any other value
- `NO_COLOR`, `NODE_DISABLE_COLORS`, or `CLAUDECODE` set non-empty, or `TERM` of `dumb`: uncolored
- otherwise each stream is colored only when it's a terminal

`CLICOLOR`, `CLICOLOR_FORCE`, and `COLORTERM` aren't read.

| exit | when                                                                                                                                                                   |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | the module ran to completion, or `--help` or `--version`                                                                                                               |
| 1    | the module threw, rejected a promise no one handled, left a top-level `await` unsettled, an import failed to resolve, load, or strip, or the runtime failed to start |
| 2    | a usage error (no arguments, an unknown subcommand or flag, an extra argument, a non-UTF-8 argument), or an entry that doesn't resolve (missing, the wrong extension, a `.js` file not under `"type": "module"`) |

An entry that fails to resolve on an I/O error or a symlink loop exits 1, not 2.
A TypeScript module nested deeper than the strip's stack can hold aborts the process (`SIGABRT`)
rather than failing the load. In a release build V8's own depth limit usually refuses it first,
with a `RangeError` and exit 1, and a depth limit in tsv closes the gap.
