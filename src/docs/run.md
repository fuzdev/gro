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
The grant flags are designed to go before the file too, in Deno's spelling, `--allow-env=A,B` and `-A` (`--allow-all`),
where a bare `--allow-env` is a usage error, not a grant of every name.
The script reads its arguments as `args`, from `import {args} from '@fuzdev/gro/args.ts'`,
the one [builtin module](glossary.md#builtin-module) built so far ([on `fgro`](host_modules.md#the-process-modules-on-fgro)).
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
- `@fuzdev/gro/args.ts`, a builtin module: the binary resolves it ahead of package lookup, reading no `package.json`,
  and grants it to the project's own modules, while a module of an installed or linked package is refused it for now
  when the graph links, the message naming the grant and the package ([resolver](resolver.md#builtin-modules) has the designed route past that).
  A package's module importing the project's own modules by name is refused too, so it can't reach the grant through one.
  `@fuzdev/gro/args.js`, the host modules not on `fgro` yet (`@fuzdev/gro/fs.ts`, `env`, `stdout`, and the rest, either spelling),
  and `@fuzdev/gro/process.ts` and `process_error.ts`, which Gro has none of (either spelling),
  are refused ahead of package lookup, as is a respelling of these or of `@fuzdev/gro/args.ts` (`@fuzdev/gro/arg%73.ts`),
  naming the specifier to import, the modules to import instead, or that the module isn't on `fgro` yet,
  never loaded from the npm package's Node implementation.
- A `./x.js` specifier whose file is missing while `./x.ts` exists is refused with a hint naming the sibling,
  as is `.mjs` beside `.mts`, and an extensionless specifier gets a hint naming the file it likely meant.
- Import attributes follow the language's rule: `type` is the one key supported,
  and an attribute under any other key is refused before anything resolves, naming the key,
  for a static import and `import()`, a file and a builtin module alike, ahead of any refusal the specifier itself would get
  (an `import()` in code compiled from a string gets that code's refusal first, below).
  An import of a builtin module with any attribute is refused before its grant is checked,
  and a file imported with a `type` is refused when it loads, whatever the value (JSON modules among them, until they land).
  No value makes a file load another way: a file whose name doesn't load as a module, like `x.txt` or `.env`, is refused with `type: 'text'` as without it.
  Each refusal is a `TypeError`, a static import's included, where the language specifies a `SyntaxError` (Node's is a `TypeError` too).
- Refused with the reason are
  any other extension (`.cjs`, which is CommonJS; `.json` and `.wasm`, not yet: JSON modules and WASM's ES module integration come later), `.cts`, `.tsx`, declaration files, Svelte modules,
  `node:`, `npm:`, `jsr:`, `https:`, every other scheme but `file:`, and `#` imports (the last comes later).
  The host's own modules (`ext:`) and the builtin modules' URLs (`builtin:`) are among the refused schemes:
  none loads into user code by a static `import`, `import()`, or `import.meta.resolve`.
  A Node built-in's bare name, like `fs`, fails as a package that isn't installed.
- An `import()` inside code compiled from a string (an `eval`, direct or indirect, a `Function` constructor, its async and generator ones included,
  or a string passed to `setTimeout` or `setInterval`) is refused with a `TypeError`, whatever it names, wherever the code is called from.
  V8 can share such code between the modules that compile the same string and names the first as its importer,
  so it has no importer of its own whose grants could be checked.
  This refusal comes before every other check of the `import()`, its attributes' included, so no message about it names a module.
  A function defined in a module imports as that module, whatever code calls it.
  The fix is to call `import()` in a module and hand what it returns to the compiled code.

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
and refused when imported as written: `import()` of `import.meta.resolve('@fuzdev/gro/args.ts')` fails.

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
When one of gro's rules about authority or containment refused it (an ungranted builtin module, a builtin module with import attributes,
a package's import of the project, a reserved specifier, a path leaving its root or reaching into `node_modules`,
a refused scheme like `ext:` or `data:`, code compiled from a string),
the refusal is reported after a run that ends, even if the program caught it, so a `try/catch` can't hide a probe for authority:
`warning: import() refused: <its message>` on stderr, its control and bidirectional formatting characters escaped (`\u{1b}`),
once per specifier, importer, and reason however often it's retried, so a second refusal at one site prints too,
for the `import()` and the imports of the modules it loads.
The report is the last thing the run prints, after its error when it fails, so nothing the program wrote can follow it to move the cursor back over it.
One that is the run's uncaught error (a `TypeError` with its message) prints only as that, unless it names a host module.
The report is bounded, and whatever else a run refuses, it lists the first refusal naming each host module by the project and by each of the first 32 packages to be refused one,
where a refusal names a host module when it's a builtin module the importer isn't granted or imports with attributes, or a reserved specifier,
and each package directory counts, one nested in another's `node_modules` included.
Past those it lists up to 100 more naming a host module (respellings, a package past the 32) and the first 100 naming none,
then `warning: import() refused: N more times for a host module, not listed` and `warning: import() refused: N more times, not listed` for the rest,
counting each refusal past them, a retry of an unlisted one included.
A listed specifier is clipped to 1 KiB where it stands, marked with its length and the head of its BLAKE3 hash, so two long ones stay apart,
an importer's URL to 512 bytes, keeping its file and the name of the first package under `node_modules` it's in, the one the project installed, when that name ends in the first 256 bytes, and what's left of the message to 4 KiB.
A refusal because nothing is there to load (no such file or package, no matching export or condition, an extension or format that doesn't load, `node:`, a registry specifier)
or because a feature isn't supported (an import attribute under a key but `type`, a file imported with a `type`)
isn't reported, so probing for an optional dependency or a feature stays quiet, and neither is `import.meta.resolve`, which imports nothing.
A reported refusal doesn't change the exit code.
A run killed by a signal, Ctrl-C included, prints no report.
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
A script can't set its own exit code on `fgro` yet; as designed, the code it sets passes through verbatim,
so `1` and `2` are shared between the script and `fgro`'s own failures.
A panic exits 101 in a debug build; a release build aborts.
`--version` or `--help` whose stdout can't be written (a pipe whose reader closed) exits 1,
naming the failure on stderr; a stderr that can't be written is ignored.
A TypeScript module nested or chained past tsv's depth limits fails to load, exit 1, with a
message naming the limit (`cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256
wire levels`), where the same JS, under V8's far deeper limit, may run.
