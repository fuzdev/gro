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
fgro run --allow-env=HOME,NO_COLOR <file>   # the env host module, with these two variables
fgro run --allow-read=../data --allow-write=out <file>   # fs reads ../data, and out/ is a write root
fgro run --allow-temp <file>   # fs.temp_dir makes fresh directories under the OS's temporary directory
fgro run --allow-run=git,cargo --allow-run-env='CARGO_*' <file>   # spawn makes these programs, each may be passed what CARGO_* covers when its pass_env names it
fgro run --allow-write=target --allow-run-in=target <file>   # spawn.program_in runs what the run builds in target/
fgro run -A <file>          # unconfined: every grant at its widest
fgro --version              # the version, the git commit, the V8 version, and whether a startup snapshot is embedded
```

`--version` prints one line, `fgro <version> (commit <sha>, V8 <version>, snapshot)`.
The commit is the full SHA the binary was built from, with `-dirty` appended when the working tree differed from it,
untracked files included and the bench's history file excepted;
`cargo xtask build` embeds it, and a binary from a plain `cargo build` embeds none (`commit unknown`).
The last part is `no snapshot` for a build without a startup snapshot.

`run` reads its own options up to the entry file, the first argument that is neither `help` nor starts with `-`,
and every argument after the file is the script's, passed as written, `--` and `--help` included.
Its options are `--help` (or `help`, as for every command) and the grant flags, in Deno's spelling,
and any other before the file is a usage error, `--allow-all` among them (`-A` is the one spelling).
A usage error that names an argument, an unknown option or one that isn't UTF-8, prints it only up to its first `=`, marked `=…`,
with control characters, the line and paragraph separators, and format characters (Unicode's `Cf`: zero-width ones, the soft hyphen, the byte order mark, the bidirectional controls) escaped as `\u{…}`,
so what follows a `=`, which could be a value, never reaches the terminal, and neither does a raw control character
(a name `--allow-env=` or `--allow-run-env=` can't grant is echoed only for a `*`, escaped, and never when it holds a `=`;
a name `--allow-run=` can't grant is a program's, echoed escaped).
Arguments the command-line parser refuses before `run` reads its own, an unknown flag of `fgro` or of `eval` among them, print as
`error: invalid arguments to fgro eval: <the arguments>` (or `to fgro:`) and `Run fgro eval --help for more information.`,
the first 16 arguments each printed by that rule and cut past 80 characters, then a count of the rest, and no other text, so the message doesn't say which argument was wrong;
a file whose name starts with `-` goes after `--`.
A run grants the project's own modules `args`, `exit`, and `fs`, whose root reads the project's tree and writes nothing, and these widen it, for that run only:

- `--allow-env=A,B` grants `@fuzdev/gro/env.ts` to the project's modules, its root handle reading exactly those variables.
  It's repeatable, the names of every one joined, and each name is exact and case-sensitive,
  held to the rule the module's `narrow` holds a name to: not empty, no NUL byte, no `=`, and no `*`, which names no glob.
  A bare `--allow-env`, an empty list, an empty name (`--allow-env=A,`), and a name breaking the rule are usage errors,
  a bare one never Deno's grant of every name.
  `fgro` reads the granted variables once, as the run starts, and the runtime reads none;
  a value that isn't UTF-8 is read with U+FFFD in place of each invalid sequence, as Node reads one.
- `--allow-read=a,/b` grants `@fuzdev/gro/fs.ts` reading these directories, and `--allow-write=a,/b` writing them,
  beside the project's root, which `fs` reads with neither and writes only when a directory holding it is written.
  Each is repeatable, its directories one set, a path relative to the working directory or absolute,
  and each must be a directory when the run starts, under `-A` too:
  a path missing, a file, or one `fgro` can't look at is a usage error naming it as resolved, before anything runs.
  One whose real path isn't its text resolved against the working directory, `.` and `..` read by their text, as a path through a symlink, is noted on stderr before the program runs:
  `note: --allow-read=<as typed> grants <real path>`, both escaped, since a path copied from a refusal is the program's text and can lead elsewhere through a symlink.
  Every symlink is noted, a symlinked home's included; the working directory is read as its real path, so a relative path through no symlink notes nothing. `-A` notes nothing.
  `fs` opens each, by the real path it had as the run started, when the run first imports `fs`, and every op through one moved or replaced before then fails ([host modules](host_modules.md#fs-on-fgro)).
  A bare flag, an empty list, an empty path (`--allow-read=a,`), and a NUL byte are usage errors too, and a path holding a comma can't be listed.
  Write doesn't imply read: a directory only written is read by no read op, though `fs.open_dir`, `dir()`, and a write can still tell whether something is there,
  and a handle's `path`, its real path, where a directory symlink in it leads.
  A directory written inside one read is read too, and a handle reads and writes as the directory it was opened on, never more,
  so a program writes a granted directory through a handle `fs.open_dir` makes on it, never through `fs.project` or a handle made from it
  ([host modules](host_modules.md#fs-on-fgro) has how a handle carries its scope).
- `--allow-temp` grants `@fuzdev/gro/fs.ts`'s `temp_dir` making fresh directories under the OS's temporary directory,
  each a root the run reads and writes, added as the run goes, and nothing else of the temporary directory, which it neither reads nor writes.
  It takes no value: `--allow-temp=` with anything after it is a usage error, its value never echoed.
  `fgro` reads the temporary directory as Node's `os.tmpdir()` names it, the first of `TMPDIR`, `TMP`, and `TEMP` set and not empty, else `/tmp`,
  once as the run starts and only for a run that grants it, so a run that doesn't learns nothing of where it is,
  and holds it to its real path then, so a symlink a run makes on its way is refused, never followed
  ([host modules](host_modules.md#fs-on-fgro) has the directory's lifecycle).
- `--allow-run=git,./bin/tool` grants `@fuzdev/gro/spawn.ts` to the project's modules, its root handle making programs of exactly those names.
  It's repeatable, the names of every one joined, and each is exact, as a program asks for it:
  a bare name is found on `PATH`, absolute entries only and never a `node_modules/.bin`, and a name holding `/` beneath the project's root.
  An empty list, an empty name, a NUL byte, a `*`, an absolute name, and one holding `/` that climbs above the project's root by `..` are usage errors, and so is a bare `--allow-run`.
  `fgro` reads what a child may be given for a run that grants a program, once, as the run starts (`PATH`, `HOME`, `TMPDIR`, `TERM`, `NO_COLOR`, `FORCE_COLOR`, and what `--allow-run-env=` names),
  and pins each name then, once the entry has resolved and before any module evaluates, to the file it finds and that file's real path ([host modules](host_modules.md#spawn-on-fgro));
  a name that finds nothing is no error, and `find` returns `null` for it.
  Until each child is boxed, granting any program is close to granting what `-A` grants:
  most programs can be made to run code by their arguments or environment, a shell, git, and cargo among them,
  and the root's `env` values set any variable for a program; hand library code a narrowed root, which refuses `env` values.
- `--allow-run-in=target,../tools` grants `@fuzdev/gro/spawn.ts` to the project's modules with these directories as exec roots, listed as `--allow-read=` lists its own:
  `spawn.program_in(dir, name)` and `find_in` find a program beneath an fs handle's directory that lies beneath one, and refuse any other directory `not_granted`.
  Each is checked to be a directory as the run starts, then opened by its real path, beside the pins, and held to the directory it named then,
  so one replaced between is refused ([host modules](host_modules.md#spawn-on-fgro)). A bare `--allow-run-in` is a usage error.
  An exec root that is also a directory `--allow-write=` grants runs what the run writes there, so a run granted both can write a program and run it:
  as for `--allow-run=`, granting one is close to granting what `-A` grants.
- `--allow-run-env=A,CARGO_*` lets the programs `--allow-run=` grants, and those found beneath what `--allow-run-in=` grants, be passed these variables, when a program's `pass_env` names them:
  each an exact name, or a `PREFIX_*` glob, which covers every name it starts and lets `pass_env` name a glob within it.
  It's repeatable, its names one set, and it's a usage error without `--allow-run=` or `--allow-run-in=`, bare, or naming what no variable can be named: an empty name, a NUL byte, a `=`, a `*` before a name's end, or `*` alone, which would pass every variable without saying so.
  `fgro` reads each exact name by name, and the whole environment only when a glob is granted, keeping what a glob covers, so the runtime holds no variable it could pass no child;
  a script reads none of them: the spawn module gives a child what its program's options name, and no member returns a value.
  A variable of the base set needs no grant: `pass_env` may name one exactly, since every child is given it.
- `-A` runs unconfined, the one flag that grants beyond what it names: every builtin module to every module of the run, a package's included,
  and every grant at its widest, so `env`'s root handle reads every variable `fgro` started with
  (each whose name is UTF-8 and holds no `=`, each value read as `--allow-env=` reads one),
  and `spawn`'s root makes any program, found when it's asked for, as Node's does, passed any variable, `pass_env: 'all'` included.
  Names, directories, and `--allow-temp` given beside it are still checked, and add nothing: it makes temporary directories too.
  It lifts no rule but a grant: a host module not built yet, a reserved specifier, a package's import of the project,
  and an `import()` in code compiled from a string stay refused, and it adds no global.
  `fs` reads and writes every directory under it, one root at `/` that resolves an absolute path as the kernel does.
The script reads its arguments as `args`, from `import {args} from '@fuzdev/gro/args.ts'`,
one of the [builtin modules](glossary.md#builtin-module) built so far ([host modules](host_modules.md#the-modules) lists them),
and ends the run or sets its exit code with `exit` and `set_exit_code`, from `@fuzdev/gro/exit.ts` ([below](#errors-and-exit-codes)).
User code can't read stdin (there is no `process` global, `Deno`, or `Bun`) and reads the environment only through `@fuzdev/gro/env.ts`, the variables `--allow-env=` or `-A` grants;
it reads the filesystem only through `@fuzdev/gro/fs.ts`, beneath the project's root, the directories `--allow-read=` names, and those `fs.temp_dir` makes under `--allow-temp` (everything under `-A`),
and writes only beneath the directories `--allow-write=` names and those `fs.temp_dir` makes under `--allow-temp`;
it starts only the programs `--allow-run=` names and those found beneath what `--allow-run-in=` names, through `@fuzdev/gro/spawn.ts` (any under `-A`), and no module gives it the network.
That is the whole of today's confinement: the process itself has no kernel box and no resource limits, the JIT is on,
and console output reaches the terminal unescaped: it's the program's own, and can imitate `fgro`'s lines, an `error:` or a `hint:` among them.
So `fgro run` is not yet for code you treat as hostile
([authority](authority.md#threat-model) and [backstop](backstop.md) have what's designed).
The event loop drains before the run ends, so timers and pending promises complete,
and a top-level `await` on a promise nothing settles or waits on ends the run as an error, not a hang;
a call's child is waited on until it ends, so a run awaiting a child that never ends waits for it, as on Node.
When the run ends, by `exit`, an uncaught error, or its event loop's end, every child it started and hasn't reaped is killed, with the process group the child leads.
A run that grants a program (`--allow-run=`, `--allow-run-in=`, or `-A`) raises its soft limit on open descriptors to the hard one as it starts, as Node does, so many children with their pipes fit,
and each child inherits the raised limit; a raise the kernel refuses leaves the limit as it was and the run going, as on Node.
Linux x86_64 only.
A binary from `cargo xtask build` starts from its embedded snapshot and runs anywhere,
while a plain `cargo build` runs only from its build tree.
The snapshot build is reproducible: a clean checkout of one commit, with the same toolchain (Rust's, and the system's C compiler and C library) and cargo home (whose registry paths the binary names), builds the same `fgro`, byte for byte, from any path.
The binary embeds the commit, so a dirty tree (a changed file, or an untracked one git doesn't ignore) builds other bytes, and so does one git can't read as its own checkout (outside git, or a copy inside another repo), which embeds none.

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
- `@fuzdev/gro/exit.ts`, a builtin module granted to the project's own modules as `args` is, and refused to a package's the same way:
  `exit(code?)` ends the process inside the call, and `set_exit_code(code)` sets the code a run that ends on its own exits with
  ([host modules](host_modules.md#exit-on-fgro)).
- `@fuzdev/gro/cwd.ts`, `@fuzdev/gro/pid.ts`, and `@fuzdev/gro/runtime_version.ts`, builtin modules that need no grant,
  so the project's modules and a package's import them alike:
  `cwd()` is the directory `fgro` was started in, `pid` the process's id, and `runtime_version` the string `fgro/<version>+<commit>`,
  the commit as `--version` prints it, and `fgro/<version>` alone in a build that embeds none ([host modules](host_modules.md#the-process-modules-on-fgro)).
  In a working directory whose path isn't UTF-8, `cwd()` throws when it's called.
- `@fuzdev/gro/env.ts`, a builtin module granted to the project's own modules only by `--allow-env=` (and to every module by `-A`):
  `env`, the root handle, reading only the variables the run was granted, and `env.narrow(names)`, a handle of a subset of its receiver's names;
  a name outside a handle's set throws `EnvError` `not_granted`, which fails the run caught or not
  ([host modules](host_modules.md#env-on-fgro)).
- `@fuzdev/gro/env_error.ts`, a builtin module that needs no grant: `EnvError` and `is_env_error`,
  the class and guard the runtime makes ([host modules](host_modules.md#error-classes-on-fgro)).
- `@fuzdev/gro/blake3.ts`, a builtin module that needs no grant: BLAKE3 over bytes by the native `blake3` crate,
  one-shot and by hashers whose state is the runtime's ([host modules](host_modules.md#blake3)).
- `@fuzdev/gro/fs.ts`, a builtin module that needs the `fs` grant: every member of the Node module,
  beneath the project's root, the directories the flags grant, and those `fs.temp_dir` makes under `--allow-temp`,
  a read refusing anything but a regular file at once; `fs.open_dir` or `fs.real_path` outside them rejects
  with `FsError` `not_granted`, which fails the run caught or not, and so does a read through a handle that doesn't read
  or a write through one that doesn't write, while a handle's reach outside its directory is `outside_root`, which doesn't ([host modules](host_modules.md#fs-on-fgro)).
- `@fuzdev/gro/fs_error.ts`, a builtin module that needs no grant: `FsError`, `is_fs_error`, and `is_own_fs_error`.
- `@fuzdev/gro/spawn.ts`, a builtin module granted to the project's own modules only by `--allow-run=` and `--allow-run-in=` (and to every module by `-A`):
  `spawn`, the root handle, making program handles of the names it may, of programs beneath the exec roots, and narrower roots, a refusal throwing `SpawnError` `not_granted`, which fails the run caught or not;
  a program's `run`, `out`, `out_bytes`, and `start` start its child, refused `launch_failed` when its file changed since it was pinned ([host modules](host_modules.md#spawn-on-fgro)).
- `@fuzdev/gro/spawn_error.ts`, a builtin module that needs no grant: `SpawnError` and `is_spawn_error`.
- A builtin module's `.js` spelling (`@fuzdev/gro/args.js`), the host modules not on `fgro` yet (`@fuzdev/gro/net.ts`, `stdin`, `stdout`, `net_error`, and the rest, either spelling),
  and `@fuzdev/gro/process.ts` and `process_error.ts`, which Gro has none of (either spelling),
  are refused ahead of package lookup, as is a respelling of these or of a builtin module's specifier (`@fuzdev/gro/arg%73.ts`),
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
  `node:`, `npm:`, `jsr:`, `https:`, every other scheme but `file:`, and a `#` import that maps to a package or builtin module rather than a file
  ([resolver](resolver.md#imports)).
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
with the fix to import `foo` by name, and that refusal fails the run even when caught ([below](#errors-and-exit-codes)).
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
Each line is escaped as a usage error's argument is ([above](#usage)), line breaks included,
and a message breaks into lines only before a frame's `at` (a module that failed to strip names its position so),
so the program's text, a message, a thrown value, or a stack it set, starts no line but a frame's.
A frame's path prints percent-decoded, so a directory with a space reads as it does on disk,
and a frame is one line up to the next, so a line break in a module's file name is escaped in it,
unless what follows it reads as a frame's start (four spaces and `at `), as in a stack V8 formats.
fgro applies no source maps, as Node doesn't by default, so a position always names the code that ran,
in a stack, a report, and a failed import's position alike (its vendored `deno_core` is patched to apply none).
TypeScript needs none, since the type strip blanks types to whitespace,
so every line and UTF-16 column is the source's.
Bundled JS that carries a map (`//# sourceMappingURL=…`) shows positions in the bundle.
A web API that fails with a `DOMException` throws one of its name, like `InvalidCharacterError` from `atob('!')`.
A module that fails to resolve, load, or strip prints its reason without `Uncaught`
(`error: cannot resolve '…' from …`), and a refused TypeScript construct names itself, a fix, and its position.
A dynamic `import()` of such a module rejects with an error the code can catch, a `TypeError` with the `code` `'ERR_MODULE_NOT_FOUND'`,
an I/O failure reading the resolved file's included.
When one of gro's rules about authority or containment refused it (an ungranted builtin module, a builtin module with import attributes,
a package's import of the project, a reserved specifier, a path leaving its root or reaching into `node_modules`,
a refused scheme like `ext:` or `data:`, code compiled from a string),
the run fails, exiting 1 where it would have exited 0, whether or not the program caught the refusal, so a `try/catch` can't hide a probe for authority,
and so does a refused import in a module the `import()` loads.
So does a call a builtin module refused for a grant its handle doesn't hold, caught or not:
an environment variable outside the names an `env` handle may read, the root's being the ones `--allow-env=` granted,
which throws `EnvError` `not_granted` and fails the run (an unset granted name, a `narrow` to a subset of the names, and an argument that is no name, a `TypeError`, don't);
or a path `fs` refuses `not_granted`: one outside every root the call may use, or an op its handle's scope lacks;
or a program `spawn` refuses `not_granted`: a name outside the set of the root it's asked of, the run's root's being the ones `--allow-run=` granted,
a variable a program's `pass_env` names that `--allow-run-env=` doesn't forward (a name of the base set needs none), `pass_env: 'all'` but under `-A`, an absolute name or one climbing above the project,
what a narrowed root refuses, and a directory no exec root of the root's scope holds, the run's root's being the ones `--allow-run-in=` granted.
After the run's own error, if it has one, stderr lists the first 8 distinct refusals of either kind in the order they happened,
`error: import() refused: <its message>` for an import and `error: env refused: not_granted: <name>`, `error: fs refused <need>: not_granted: <path>`, or `error: spawn refused: not_granted: <program>` for a call, never a value,
escaped as a usage error's argument is (`\u{1b}`),
and a message past 2 KiB cut there and marked with its length,
then counts the rest, a retry of an unlisted one included: `error: N more refusals, not listed`,
then closes with `error: a refusal fails the run, whether or not the program caught it`.
One that is the run's uncaught error (a `TypeError`, an `EnvError`, an `FsError`, or a `SpawnError` with its message) prints as that and isn't listed, though past the listed ones it's still counted;
an `FsError` names no need, so when `fs` refused its path more than once, needing different grants, each of those stays listed,
and once any refusal was counted, the uncaught one may be among the counted, so every listed one stays.
Last comes one hint, `hint: add --allow-env=A,B to grant what env refused`, naming each variable the run's own grant lacked,
distinct, in the order each was first refused, the first 8, its uncaught one's and the counted ones' included:
a name the root `env` refused, which holds what `--allow-env=` gave the project, so the flag lets the call through, whichever module made it.
It names only ASCII letters, digits, and `_`, which every shell passes as printed, since the program chose the names and pasting the hint mustn't run anything,
and reads `some of what env refused` when it can't name them all: past 8 names, a name longer than 2 KiB, or a name of any other character
(a comma, which separates the flag's names, a glob, a space, a quote, a shell's `$`, a control character, a lookalike letter).
With no name it can show, there's no hint line, so a run whose first 8 refused names all fall outside these characters prints none.
Then, by the same rules, `hint: add --allow-run=a,b to grant what spawn refused`, naming each program the run's root refused,
only a plain name, of ASCII letters, digits, `.`, `_`, `+`, and `-`, starting with a letter or a digit, never a name holding `/`, which is a path the program chose;
and `hint: add --allow-run-env=A,B to grant what spawn refused`, naming each variable a granted program's `pass_env` named that the flag doesn't forward, by `env`'s rule, so never a glob,
whichever root passed it on, a narrowed one included, since the flag widens the program's grant.
Each counts its own names, and reads `some of what spawn refused` when it can't name them all of its kind.
Then `hint: add --allow-run-in naming the directory to grant what spawn refused`, once, for a directory the run's root found in no exec root, fixed text naming no path, as `fs`'s hints are.
After them come the hints for `fs`, each once when a refusal needed it, in this order, each fixed text that names no path:
`hint: add --allow-read naming the directory to grant what fs refused`, for a reach outside every root or a read through a handle that doesn't read;
`hint: fs refused a symlink leaving its root (an absolute target, or a .. above the root, even one leading back in): make the link relative within its root, or open its target's directory by its own path with fs.open_dir`,
for a reach that left every root holding its path through a symlink whose target leaves the root by its text ([host modules](host_modules.md#fs-on-fgro)),
which a grant of the target's directory needn't let through, since a root holds a path by its text, not by where a link leads
(a target in no root needs that grant before its directory opens, which a read refusal of it then hints);
`hint: add --allow-write naming the directory to grant what fs refused, then open it with fs.open_dir and write through that handle`, for a write through a handle that doesn't write,
since a handle never gains what its root lacked, so `fs.project` writes nothing, whatever is granted inside it;
and, when a root already grants what was refused, holding by its text the path a read reads or each path a write writes (a copy's `to`, a rename's `from` and `to`), `hint: the run grants what fs refused: open the directory with fs.open_dir and use that handle`,
never a flag, which would widen the handle's root (the whole project, for a write through `fs.project`);
and `hint: add --allow-temp to grant what fs refused`, for an `fs.temp_dir` the run doesn't grant.
A copy is refused naming the path it writes, its `to`, and judged by it, so a copy into a directory written from outside it hints the handle to open, which writes the copy but can't name its source: the program reads through one handle and writes through the other.
A walk opens the names before a `..` first, so `fs.real_path('/usr/../etc')` needs `/usr` granted as well as `/etc`.
An fs refusal's `<need>` is the kind of grant it needed, in its hint's words:
`a read`, `a symlink leaving its root`, `a write`, `what the run grants` (a handle to open), `a temporary directory`, or `what no grant allows`,
for a `remove` or `rename`, through a handle that doesn't write, of a path that is, or holds, a root's directory by its text, which the rule keeping each root's directory in place refuses whatever the run grants ([host modules](host_modules.md#fs-on-fgro)),
so a hint pairs with the listed refusals of its kind or with none listed, never with another kind's,
and two refusals of one path that needed different kinds are two lines.
The listed refusal names the path; a hint never does, since a path the program chose, shown back, can grant another directory than it names, through a symlink, or run something when pasted.
There's no hint for a name outside a narrowed handle's set, which no flag widens, for any refusal under `-A`, whose roots reach everything,
for `pass_env: 'all'`, which no flag grants,
or for a refused `import()`, which records no grant: a package is granted no host module by any flag, most rules are grantable by nothing,
and the project's `import()` of `env` without `--allow-env=` names no variable the flag could list.
The hint is made from what the run recorded of each refusal, the grant that would have allowed it, never from the listed text.
A refusal because nothing is there to load (no such file or package, no matching export or condition, an extension or format that doesn't load, `node:`, a registry specifier)
or because a feature isn't supported (an import attribute under a key but `type`, a file imported with a `type`)
doesn't fail the run and isn't listed, so probing for an optional dependency or a feature still exits 0, and neither does `import.meta.resolve`, which imports nothing.
A run that a signal kills, Ctrl-C included, or that aborts, as V8 does out of memory, lists nothing, and never exits 0.
A module whose file was replaced between resolving and loading isn't loaded
(`cannot load <url>: the file at its path is not the one resolved, so it isn't loaded`;
[resolver](resolver.md#departures-from-node) has what it catches and what it can't),
the entry's included, which exits 1.

`console.log` and `console.info` write to stdout, `console.warn` and `console.error` to stderr.
A write whose reader has left, a run piped to `head`, is dropped, and the run goes on to its own exit code, so a run that never ends on its own runs until signalled;
any other failed write, like one to a full disk, throws an `Error` naming the stream, where Node's console drops it.
Whether a stream is colored is decided once per process, in order:

- `FORCE_COLOR` set, even empty: colored for `''`, `1`, `2`, `3`, or `true`, as in Node, and uncolored for any other value
- `NO_COLOR`, `NODE_DISABLE_COLORS`, or `CLAUDECODE` set non-empty, or `TERM` of `dumb`: uncolored
- otherwise each stream is colored only when it's a terminal

`CLICOLOR`, `CLICOLOR_FORCE`, and `COLORTERM` aren't read.

| exit           | when                                                                                                                                                         |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 0              | the module ran to completion with no code set, or called `exit()` with none set, or `--help` or `--version`                                                  |
| the program's  | the code given to `exit(code)`, or the one `set_exit_code` set when the module then ran to completion or called `exit()` with none: an integer from 0 to 255 |
| 1              | the module threw, rejected a promise no one handled, left a top-level `await` unsettled, an import failed to resolve, load, or strip, or the runtime failed to start, or a run that grants a program couldn't make ready to start one, whatever code `set_exit_code` set; or an `import()` was refused by one of gro's rules, or a call for a grant its handle lacks, caught or not, in a run that would have exited 0 |
| 2              | a usage error (no arguments, an unknown subcommand or flag, a flag before `run`'s file other than `--help`, `--allow-env=`, `--allow-read=`, `--allow-write=`, `--allow-temp`, `--allow-run=`, `--allow-run-in=`, `--allow-run-env=`, or `-A`, a bare one of these or a name it can't grant, `--allow-temp` given a value, `--allow-run-env=` without `--allow-run=` or `--allow-run-in=`, a directory missing, no directory, or one that can't be looked at, no file, an extra argument to `eval`, a non-UTF-8 argument, the script's included, `--version` with a subcommand), or an entry that doesn't resolve (missing, the wrong extension, a `.js` file not under `"type": "module"`) |

An entry that fails to resolve on an I/O error or a symlink loop exits 1, not 2.
A program's code passes as it is, so `1` and `2` are shared between the program and `fgro`'s own failures.
`exit` ends the process inside the call: no `finally` block runs, no queued job, no timer, and an error then unwinding isn't reported.
An `import()` or a call refused by rule turns a `0` into `1` however the run ended, `exit(0)` included, and leaves any other code as it is,
and its lines are printed before the process ends.
An `import()` whose graph is still loading when `exit` is called is abandoned where it is:
a refusal the graph would meet later is never reached, so it neither fails the run nor prints.
A top-level `await` nothing settles or waits on exits 1, where Node exits 13.
A `SIGINT`, `SIGTERM`, or `SIGHUP` that ends a run that started a child ends it by that signal, as with none,
once the groups its children lead are killed and a `SIGTERM` is passed on to its children in its own group,
a child launching as the signal arrives among them, waited for up to a second;
a `SIGTERM` that arrives with a `SIGINT` or `SIGHUP` that ends the run first isn't passed on.
A panic exits 101 in a debug build; a release build aborts.
`--version` or `--help` whose stdout can't be written (a pipe whose reader closed) exits 1,
naming the failure on stderr; a stderr that can't be written is ignored.
A TypeScript module nested or chained past tsv's depth limits fails to load, exit 1, with a
message naming the limit (`cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256
wire levels`), where the same JS, under V8's far deeper limit, may run.
