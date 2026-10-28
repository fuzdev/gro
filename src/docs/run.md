# run

> `fgro run <file>` as built today: what it loads, what the global scope holds,
> how a run fails, and the exit codes

> **status**: built; the reference for the current binary, which has no release yet.
> The README's [Develop](../../README.md#develop) section says how to build it,
> and [runtime](runtime.md) has the design it grows into.

## Usage

```bash
fgro run <file>   # a .js, .mjs, .ts, or .mts module, relative to the working directory or absolute
fgro --version    # the version, the V8 version, and whether a startup snapshot is embedded
```

`run` takes only the entry file: an extra argument is a usage error, there are no script arguments,
and nothing reads stdin or the environment (there is no `process`, `Deno`, or `Bun`).
The event loop drains before the run ends, so timers and pending promises complete,
and a top-level `await` on a promise that never settles ends the run as an error, not a hang.
Linux x86_64 only.

## What loads

- `.mjs`, `.ts`, and `.mts` modules, and `.js` modules under a `package.json` with `"type": "module"`;
  a `.js` file without one is refused, never sniffed.
  TypeScript is stripped of its types in process ([typescript](typescript.md)).
- Relative and absolute specifiers with their full extension,
  and bare specifiers through a package's `"exports"` in `node_modules` ([resolver](resolver.md)).
- A `./x.js` specifier whose file is missing while `./x.ts` exists is refused with a hint naming the sibling.
- Refused with the reason: JSON modules and other import attributes, `.cts`, `.tsx`, declaration files,
  Svelte modules, `node:`, `npm:`, `jsr:`, `https:`, and `#` imports (the last comes later).

## The global scope

The ECMAScript globals, including `Intl`, `Temporal`, and `WebAssembly`, plus these web globals:

- `console`, `queueMicrotask`, `structuredClone`, `performance`
- `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`
- `URL`, `URLSearchParams`, `URLPattern`
- `TextEncoder`, `TextDecoder`, `TextEncoderStream`, `TextDecoderStream`, `atob`, `btoa`
- `ReadableStream`, `WritableStream`, `TransformStream`, their readers, writers, controllers, and queuing strategies
- `Blob`, `File`, `CompressionStream`, `DecompressionStream`
- `AbortController`, `AbortSignal`, `Event`, `EventTarget`, `CustomEvent`, `ErrorEvent`, `PromiseRejectionEvent`, `DOMException`

Absent today, by design until a grant or a consumer brings them: `fetch`, `Headers`, `Request`, `Response`,
`WebSocket`, `crypto`, `self`, `navigator`, `reportError`, `Worker`, and web storage.

`import.meta` has `url`, `main`, `resolve` (one argument), `filename`, and `dirname`.

## Errors and exit codes

Errors print to stderr prefixed `error:`.
An uncaught error prints `Uncaught Error: message` (a module's top-level throw reads `Uncaught (in promise)`),
then V8's frames as `at file:///path:line:column`, then each `cause` as `Caused by:` with its frames;
TypeScript positions are the source's, with no source map.
A refused TypeScript construct names itself, a fix, and its position.
Console output is colored only on a terminal, and never with `NO_COLOR` or `CLAUDECODE` set non-empty.

| exit | when                                                                                                                                                                  |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | the module ran to completion, or `--help` or `--version`                                                                                                              |
| 1    | the module threw, rejected a promise no one handled, left a top-level `await` unsettled, or an import failed to resolve, load, or strip                                |
| 2    | a usage error: no arguments, an extra argument, an unknown flag, or an entry that doesn't resolve (missing, the wrong extension, a `.js` file not under `"type": "module"`) |

A TypeScript module nested deeper than the strip's stack aborts the process rather than failing the load;
a depth limit in tsv closes that.
