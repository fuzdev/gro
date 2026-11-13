# run

> `fgro run <file>` as built today: what it loads, what the global scope holds,
> how a run fails, and the exit codes

> **status**: built; the reference for the current binary.
> [Install](runtime.md#install) says how to install a release,
> the README's [Develop](../../README.md#develop) section how to build it,
> and [runtime](runtime.md) has the design it grows into.

## Usage

```bash
fgro run <file> [args...]   # a .js, .mjs, .ts, or .mts module, relative to the working directory or absolute
fgro --version              # the version, the git commit, the V8 version, and whether a startup snapshot is embedded
```

`--version` prints one line, `fgro <version> (commit <sha>, V8 <version>, snapshot)`.
The commit is the full SHA the binary was built from, with `-dirty` appended when the working tree differed from it,
untracked files included and the bench's history file excepted;
`cargo xtask build` embeds it, and a binary from a plain `cargo build` embeds none (`commit unknown`).
The last part is `no snapshot` for a build without a startup snapshot.

`run` reads its own options up to the entry file, the first argument that is neither `help` nor starts with `-`,
and every argument after the file is the script's, passed as written, `--` and `--help` included.
Its only option is `--help` (or `help`, as for every command), and any other before the file is a usage error;
a file whose name starts with `-` goes after `--`.
The script reads them as `process.args`, from `import {process} from '@fuzdev/gro/process.ts'`,
a [builtin module](glossary.md#builtin-module) carrying nothing else yet ([process on `fgro`](host_modules.md#process-on-fgro)).
User code can't read stdin or the environment (there is no `process` global, `Deno`, or `Bun`), and no module gives it the filesystem, the network, or a process.
That is the whole of today's confinement: the process itself has no kernel box and no resource limits, the JIT is on,
and console output reaches the terminal unescaped, so `fgro run` is not yet for code you treat as hostile
([authority](authority.md#threat-model) and [backstop](backstop.md) have what's designed).
The event loop drains before the run ends, so timers and pending promises complete,
and a top-level `await` on a promise that never settles ends the run as an error, not a hang.
Linux x86_64 only.
A binary from `cargo xtask build` starts from its embedded snapshot and runs anywhere,
while a plain `cargo build` runs only from its build tree.
The snapshot build is reproducible: the same sources, toolchain, and cargo home (whose registry paths the binary names) build the same `fgro`, byte for byte, from any checkout.

## What loads

- `.mjs`, `.ts`, and `.mts` modules, and `.js` modules under a `package.json` with `"type": "module"`.
  A `.js` file without one is refused, never sniffed.
  A module larger than 16 MiB is refused as it's read, before any of it is parsed or stripped.
  A world-writable directory (other-write, like `/tmp`) ends the search for it, and for every `package.json`:
  no `package.json` or `node_modules` in it is read,
  and a package installed through one is refused ([resolver](resolver.md#departures-from-node) lists the reads it refuses).
  TypeScript is stripped of its types in process ([typescript](typescript.md)).
- Relative and absolute specifiers with their full extension,
  and bare specifiers through a package's `"exports"` in `node_modules`,
  searched from the importer up to its project root ([resolver](resolver.md)).
  Every module stays inside its project's or package's root, checked on its real path,
  and each refusal names the path, the root, and a fix
  ([resolver](resolver.md#departures-from-node) has the rules,
  and [roots and origins](resolver.md#roots-and-origins) an entry inside `node_modules`).
  A module's `import.meta.url` is its real path, after symlinks.
- `@fuzdev/gro/process.ts`, a builtin module: the binary resolves it ahead of package lookup, reading no `package.json`,
  and grants it to the project's own modules, while a module of an installed or linked package is refused it
  when the graph links, the message naming the grant and the package ([resolver](resolver.md#builtin-modules)).
- A `./x.js` specifier whose file is missing while `./x.ts` exists is refused with a hint naming the sibling,
  as is `.mjs` beside `.mts`, and an extensionless specifier gets a hint naming the file it likely meant.
- Refused with the reason are a file imported with a `type` import attribute
  (JSON modules among them, until they land; an attribute under any other key is ignored for now, though the language requires its refusal)
  and a static import of a builtin module with any attribute,
  any other extension (`.cjs`, which is CommonJS; `.json` and `.wasm`, not yet: JSON modules and WASM's ES module integration come later), `.cts`, `.tsx`, declaration files, Svelte modules,
  `node:`, `npm:`, `jsr:`, `https:`, every other scheme but `file:`, and `#` imports (the last comes later).
  The host's own modules (`ext:`) and the builtin modules' URLs (`builtin:`) are among the refused schemes:
  none loads into user code by a static `import`, `import()`, or `import.meta.resolve`.
  A Node built-in's bare name, like `fs`, fails as a package that isn't installed.
- An `import()` inside code compiled from a string by an indirect `eval` or `new Function`
  is attributed to the module that first compiled that exact string, which may not be the caller
  (a direct `eval` keeps its caller),
  so such an `import()` doesn't yet separate a package's reach from the project's.

## The global scope

The ECMAScript globals, including `Intl`, `Temporal`, and `WebAssembly`, plus these web globals:

- `console`, `queueMicrotask`, `structuredClone`, `performance`
- `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`
- `URL`, `URLSearchParams`, `URLPattern`
- `TextEncoder`, `TextDecoder`, `TextEncoderStream`, `TextDecoderStream`, `atob`, `btoa`
- `ReadableStream`, `WritableStream`, `TransformStream`, their readers, writers, controllers, and queuing strategies
- `Blob`, `File`, `CompressionStream`, `DecompressionStream`
- `AbortController`, `AbortSignal`, `Event`, `EventTarget`, `CustomEvent`, `ErrorEvent`, `PromiseRejectionEvent`, `DOMException`

Absent by design are the [authority globals](glossary.md#authority-globals), which would reach outside the run:
`fetch` is `net.fetch` on the `net` host module (built as its Node implementation, a builtin module on `fgro` once `net` lands),
`WebSocket` and `EventSource` would join it there when a consumer needs them,
and any other web API that carries authority is an import too, if Gro implements it.
Absent until a consumer needs them, with no grant involved, are `Headers`, `Request`, `Response`,
`crypto`, `self`, `navigator`, `location`, `reportError`, `MessageChannel`,
`MessagePort`, and `FileReader`.
The global object isn't an `EventTarget`, so there's no `addEventListener('unhandledrejection')`.
It's still extensible, and a free `fetch` is a plain `ReferenceError` with no hint;
the seal is designed ([authority](authority.md#by-construction-or-by-default)).

`import.meta` has `url`, `main`, `resolve` (one argument), `filename`, and `dirname`.
`import.meta.resolve` runs the same strict resolver as an import,
so unlike the web's it throws a `TypeError` for a file that's missing or refused.
It admits no package and records nothing, so the URL it returns for a package is only a path.
For an installed package, `import(import.meta.resolve('foo'))` is refused as a path into `node_modules`,
with the fix to import `foo` by name.
A linked package's URL is a path like any other:
an in-tree one loads as the project's file, since its package was never admitted around it,
and an out-of-tree one is outside the project's root, refused.
A builtin module's URL is its specifier in the `builtin:` scheme, returned to any importer, granted or not,
and refused when imported as written: `import()` of `import.meta.resolve('@fuzdev/gro/process.ts')` fails.

## Errors and exit codes

Errors print to stderr prefixed `error:`.
An uncaught error prints `Uncaught Error: message`, read `Uncaught (in promise)` for a module's top-level throw
or an unhandled rejection, then V8's frames as `at file:///path:line:column`,
then each `cause` as `Caused by:` with its frames.
A frame's path prints percent-decoded, so a directory with a space reads as it does on disk.
fgro applies no source maps, as Node doesn't by default, so a position always names the code that ran,
in a stack, a report, and a failed import's position alike (its vendored `deno_core` is patched to apply none).
TypeScript needs none, since the type strip blanks types to whitespace,
so every line and UTF-16 column is the source's.
Bundled JS that carries a map (`//# sourceMappingURL=…`) shows positions in the bundle.
A web API that fails with a `DOMException` throws one of its name, like `InvalidCharacterError` from `atob('!')`.
A module that fails to resolve, load, or strip prints its reason without `Uncaught`
(`error: cannot resolve '…' from …`), and a refused TypeScript construct names itself, a fix, and its position.
A dynamic `import()` that fails rejects with an error the code can catch, a `TypeError`,
an I/O failure reading the resolved file's included.
A module whose file was replaced between resolving and loading isn't loaded
(`cannot load <url>: the file at its path is not the one resolved, so it isn't loaded`;
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
| 2    | a usage error (no arguments, an unknown subcommand or flag, a flag before `run`'s file other than `--help`, no file, an extra argument to `eval`, a non-UTF-8 argument, the script's included, `--version` with a subcommand), or an entry that doesn't resolve (missing, the wrong extension, a `.js` file not under `"type": "module"`) |

An entry that fails to resolve on an I/O error or a symlink loop exits 1, not 2.
A panic exits 101 in a debug build; a release build aborts.
`--version` or `--help` whose stdout can't be written (a pipe whose reader closed) exits 1,
naming the failure on stderr; a stderr that can't be written is ignored.
A TypeScript module nested or chained past tsv's depth limits fails to load, exit 1, with a
message naming the limit (`cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256
wire levels`), where the same JS, under V8's far deeper limit, may run.
