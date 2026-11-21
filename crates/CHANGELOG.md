# fgro changelog

> `fgro`, gro's experimental JS runtime binary, isn't released on its own:
> the first published version will be Gro running on this runtime,
> and until then this file records what the binary built from `main` does

## Unreleased

`fgro` is a Rust binary on V8 using Deno's web-standard libraries,
for Linux x86_64 only, built from source ([install](../src/docs/runtime.md#install)).

- `fgro run <file>` runs a JS or TypeScript module from disk.
  Every argument after the file is the script's, passed as written.
  It checks what a program reaches inside the process only: the process has no kernel box and no resource limits, the JIT is on,
  and console output reaches the terminal unescaped, so `fgro run` isn't yet for code you treat as hostile.
  - A program reaches the OS only through the builtin host modules, imports under `@fuzdev/gro/*` that the binary resolves ahead of package lookup,
    each with the exports and types of its Node module in the npm package.
    A run grants the ones that need a grant to the project's own modules only, `env` only under `--allow-env=` and `spawn` only under `--allow-run=` or `--allow-run-in=`:
    a package's module is refused one when the graph links,
    and can't import the project's own modules by name to reach it through one, and `fgro eval` imports no builtin module.
    A builtin module's implementation is compiled and run when a run first imports it, so a run that imports none pays for none.
    A handle one makes is the runtime's, recognized by a mark no program can read or forge.
  - `@fuzdev/gro/args.ts`: the script reads its arguments as `args`.
  - `@fuzdev/gro/exit.ts`: `exit(code?)` ends the process inside the call, with no `finally` and no queued job run,
    and `set_exit_code(code)` sets the code a run that ends on its own exits with, each code an integer from 0 to 255 passed as it is.
  - `@fuzdev/gro/env.ts`, granted by `fgro run --allow-env=A,B <file>`: its root `env` reads exactly those variables, read once as the run starts,
    and a name outside a handle's set throws `EnvError` `not_granted`.
  - `@fuzdev/gro/fs.ts`: the filesystem, through directory handles.
    A run reads the project's root with no flag, as `fs.project`,
    and `--allow-read=a,/b` grants reading more directories, `--allow-write=a,/b` writing them,
    each a directory when the run starts and held to the directory its path named then;
    one whose real path isn't its text, a path through a symlink, is noted on stderr before the program runs
    (`note: --allow-read=<as typed> grants <real path>`, and so for `--allow-write=`).
    Every member of the Node module is built: `fs.project`, `fs.open_dir`, `fs.real_path`, `fs.temp_dir`,
    and on a handle `path`, `dir`, `readonly`, `stat`, `exists`, the reads,
    the sync ops (`stat_sync`, `exists_sync`, `read_text_sync`), `read_link`, `list`, sorted by name,
    and the writes `write`, `mkdir`, `remove`, `rename`, `copy`, `set_times`, `set_mode`, and `symlink`.
    No `remove` or `rename` moves or removes a directory of the run's, or one holding one, and no `rename` moves an entry
    where the run's grants allow more than where it was (a write root's file into a directory it reads), each `io_error`.
    `--allow-temp` grants `fs.temp_dir`, a fresh directory under the OS's temporary directory, read and written,
    removed with everything in it when its handle is disposed of (`await using`).
    A handle reads and writes as the directory it was opened on and nothing widens it,
    so a program writes a granted directory through a handle `fs.open_dir` makes on it, never through `fs.project`.
    A reach outside the run's directories, or an op its handle's scope lacks, throws `FsError` `not_granted`.
  - `@fuzdev/gro/spawn.ts`, granted by `fgro run --allow-run=git,cargo <file>`: its root `spawn` makes handles of exactly those programs,
    each found on `PATH`, or beneath the project's root for a name holding `/`, and pinned as the run starts, before any module evaluates.
    `program`, `find`, and `narrow` are built, and `--allow-run-env='CARGO_*'` lets a program's `pass_env` name those variables
    beside the base set every child is given; `fgro` reads them as the run starts, and no member returns one.
    A program's `run`, `out`, and `out_bytes` start its child, a native binary by the descriptor it was pinned with and a script by its path,
    each refused `launch_failed` when its file changed since it was pinned, even in place;
    the child is waited for by its pidfd, and a group it leads is killed, by an id still its own, at the drain bound, past `max_bytes`, and after a timeout's grace.
    A call resolves to how its child ended, `exited` with its code, `signaled` with the signal's name and number, or `timed_out` with the code or the signal's name and number it ended by.
    No child outlives its run: `exit`, an uncaught error, the run's end, and a `SIGINT`, `SIGTERM`, or `SIGHUP` kill each one left, with its group,
    a child launching as the signal arrives among them, waited for up to a second.
    `start` starts a long-lived child, its `SpawnChild` holding its `pid`, its `closed`, which never rejects, its piped `stdin`, written and closed in order,
    and its piped `stdout` and `stderr`, each an async iterable once, of chunks buffered up to 1 MiB ahead of its reader;
    `kill` sends a signal by name, and a disposal stops it, and the kill at the run's end, the guards, the drain bound, and `timeout_ms` cover it as they do `run`'s.
    `fgro run --allow-run-in=target <file>` grants exec roots, each held as the run starts: `program_in` and `find_in` find a program beneath an fs handle's directory within one,
    resolved beneath the exec root's own descriptor, so an absolute name, a `..` above the directory, and a symlink leaving it are refused, and pinned as a granted program is.
    An exec root that is also a write root runs what the run writes there.
    A run that grants a program raises its soft limit on open descriptors to the hard one as it starts, as Node does, and goes on when the kernel refuses the raise.
    A call's `signal` option isn't built: given, it rejects saying so.
    Until each child is boxed, granting any program is close to granting what `-A` grants, since most programs can be made to run code
    by their arguments or environment and the root's `env` values set any variable; hand library code a narrowed root, which refuses them.
    A program outside a handle's set, a directory outside every exec root, or a variable its program's grant doesn't forward, throws `SpawnError` `not_granted`.
  - More builtin modules need no grant, so a package's modules import them as the project's do:
    `@fuzdev/gro/cwd.ts`, whose `cwd()` is the directory `fgro` was started in,
    `@fuzdev/gro/pid.ts`, whose `pid` is the process's id,
    `@fuzdev/gro/runtime_version.ts`, whose `runtime_version` is `fgro/<version>+<commit>`, the commit the binary was built from,
    `@fuzdev/gro/blake3.ts`, BLAKE3 over bytes, one-shot and by hashers,
    and `@fuzdev/gro/tsv.ts`, tsv's formatters for TypeScript, Svelte, and CSS, its import parse, and its ignore stacks, by tsv's own crates.
    A panic in tsv, a bug of tsv's, ends the process.
  - `@fuzdev/gro/env_error.ts`, `@fuzdev/gro/fs_error.ts`, `@fuzdev/gro/spawn_error.ts`, `@fuzdev/gro/stdio_error.ts`, and `@fuzdev/gro/net_error.ts` need none either: each error class and its guards are made by the runtime, natively,
    the first time a run needs them, and match the Node module's but that a guard runs no code of a caller's.
  - `fgro run -A <file>` runs unconfined: every builtin module is granted to every module of the run, a package's included,
    each grant at its widest, every variable for `env`, one root at `/` for `fs`, and `fs.temp_dir`, and any program for `spawn`, passed any variable,
    while every refusal but a grant's stands (a host module not on `fgro` yet, a package's import of the project, and the like).
  - Text `fgro` didn't write that it prints in the run's uncaught error and its frames, a refusal, a usage error, or a note is escaped
    (control characters, line breaks, and format characters as `\u{…}`), so such text starts no line but a stack frame's.
  - A refusal fails the run, even when the program catches it, turning an exit 0 into 1:
    a `not_granted` from `env`, `fs`, or `spawn`, and an `import()` refused by one of gro's rules
    (an ungranted builtin module, a reserved specifier, a refused scheme, and the like).
    Stderr lists the first distinct refusals after the run's error, escaped and cut to a bound, and counts the rest,
    closes the list with a line saying a refusal fails the run,
    then hints at the flag that would grant what was refused: `--allow-env=` with the names the project's grant lacked,
    `--allow-run=` and `--allow-run-env=` with the programs and variables, each a plain name, and for `fs` the flag alone (`--allow-read`, `--allow-write`, or `--allow-temp`) or the handle to open, never a path.
    An `import()` refused because nothing is there, like a missing optional dependency, or because a feature isn't supported, doesn't fail the run.
  - The host modules not on `fgro` yet are refused, never loaded from the npm package:
    `@fuzdev/gro/net.ts`, `watch.ts`, `stdin.ts`, `stdout.ts`, and `stderr.ts`.
    So is a builtin module's `.js` spelling (`@fuzdev/gro/args.js`), and a respelling of any of these or of a builtin module's specifier (`@fuzdev/gro/arg%73.ts`).
    So are `@fuzdev/gro/process.ts` and `@fuzdev/gro/process_error.ts`, which Gro doesn't have, either spelling,
    the refusal naming the modules to import instead.
  - An `import()` in code compiled from a string (`eval`, a `Function` constructor, a string timer) is refused,
    since V8 can share that code between the modules compiling the same string, so it has no importer whose grants could be checked.
  - Import attributes follow the language's rule: an attribute under any key but `type` is refused before anything resolves,
    for a static import and `import()` alike, and an import of a builtin module with any attribute is refused, `import()`'s included.
  - Resolution is strict: mandatory extensions, packages only through their `"exports"`,
    `.js` only under `"type": "module"`, and every module kept inside its project's or package's root.
    TypeScript is stripped of its types in process, and TypeScript with runtime semantics, like an `enum`, is refused.
  - The global scope holds the ECMAScript globals and a set of web-standard ones (no `fetch` or `crypto`),
    with no `process` global, `Deno`, or `Bun`: code reads the environment and the filesystem only through their host modules,
    can't read stdin, and no module gives it the network yet.
- `fgro eval <module>` evaluates a project's module to JSON data,
  printed as one envelope on stdout that a caller like zap reads:
  the default export, another with `--export`, or what a driver read from stdin with `--driver -` returns,
  with the module graph the run loaded when asked (`--graph`).
  It runs jitless by default (`--jit` turns on the JIT and WebAssembly),
  with no code generation from strings and `determinism: deny`,
  which refuses the clock, randomness, timers, and facts of the machine.
  Its heap, memory, time, module sources, and output are capped.
  Before any of the module's code or V8 runs, the process closes the descriptors it inherited,
  clears its environment but `TZ`, and goes into a kernel box:
  `no_new_privs`, Landlock (read-only to the project and the packages linked into it,
  no writes or execution anywhere, no TCP), and two seccomp filters,
  one killing the process on calls like `execve` and `ptrace`,
  and an allowlist that fails every other call.
  It needs Linux 6.12 or later (Landlock ABI 6) and `TZ=UTC`, and refuses to run without them.
- `fgro --version` prints the version, the commit it was built from, the V8 version,
  and that the startup snapshot is embedded.
