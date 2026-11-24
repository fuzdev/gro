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
  - `@fuzdev/gro/stdout.ts` and `@fuzdev/gro/stderr.ts`, granted like `args`: `write(data)` writes a string or bytes whole before it returns,
    through the path the console writes by, so output keeps program order and `exit` drops nothing,
    resolving `false` when the reader has gone and rejecting with `StdioError` `io_error` for any other failure;
    `is_terminal` is read once, when the run first imports the module. The console isn't gated by either grant.
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
    No child outlives its run's `exit`, an uncaught error, or the run's end, each killing every child left with its group;
    a `SIGINT`, `SIGTERM`, or `SIGHUP` that ends the run kills the groups its children lead, and a `SIGTERM` is also passed on to its children in its own group,
    a child launching as the signal arrives among them, waited for up to a second.
    `start` starts a long-lived child, its `SpawnChild` holding its `pid`, its `closed`, which never rejects, its piped `stdin`, written and closed in order,
    and its piped `stdout` and `stderr`, each an async iterable once, of chunks buffered up to 1 MiB ahead of its reader;
    `kill` sends a signal by name, and a disposal stops it, and the kill at the run's end, the guards, the drain bound, and `timeout_ms` cover it as they do `run`'s.
    `fgro run --allow-run-in=target <file>` grants exec roots, each held as the run starts: `program_in` and `find_in` find a program beneath an fs handle's directory within one,
    resolved beneath the exec root's own descriptor, so an absolute name, a `..` above the directory, and a symlink leaving it are refused, and pinned as a granted program is.
    A script at a `node_modules/.bin` name never runs by its path.
    Its `#!` line must name `node` through `/usr/bin/env`, giving only `node`'s flags,
    and it runs under a `node` found as a bare name is, with every `node_modules/.bin` entry skipped, pinned along with the script.
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
  - Gro's library loads from the binary's own copy: any other `@fuzdev/gro` specifier (`@fuzdev/gro/task_args.ts`, `@fuzdev/gro` alone its `index.ts`)
    resolves to it at its `gro:lib/` URL, never to `node_modules` or a self-reference, holding no grant by its own imports, `-A` or not,
    and takes its packages from the project root's `node_modules` at the ranges of Gro's peer dependencies, refusing one missing or out of range naming the install that fixes it.
    `internal/` modules, the ones that run only on Node, and a path Gro has no module at are refused, naming the module;
    `fgro eval` holds no copy, so it refuses every one but the builtin modules.
  - The host modules not on `fgro` yet are refused, never loaded from the npm package:
    `@fuzdev/gro/net.ts`, `watch.ts`, and `stdin.ts`.
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
- `fgro --version` prints Gro's version, `package.json`'s, the commit it was built from, the V8 version,
  and that the startup snapshot is embedded.
- `fgro --help` and `fgro help` print the binary's own help.
- `fgro <command> [<args>]`, for every other first argument and for none, runs a command of Gro's
  (`fgro check`, `fgro gen --check`; bare `fgro` lists them, and `fgro -v` prints Gro's version)
  from Gro's own code embedded in the binary.
  Before any JS runs it reads the command's grants from the grants table it carries, joins the grant flags right after the command word,
  and starts from an entry it generates, packages matching the `development` condition unless `NODE_ENV` is `production`.
  A program the table grants takes the table's environment policy.
  A forwarded section turning off a check the command's invoke runs, like `fgro check -- gro gen --no-check`, is refused before anything runs, exit 2.
  A step the command line turns off, like `fgro check --no-test`, leaves the command's grants, its tools neither pinned nor granted, and their tool directories not made.
  - The command runs in a kernel box with every tool it starts ([box](../src/docs/box.md#what-a-commands-box-admits)):
    the project read-only but for the command's write roots (a rule per top-level entry but `.git`, `node_modules`, and the other dot-entries under `gen`, `format`, and `sync`,
    and the tool directories, like `.svelte-kit` and `node_modules/.vite`, under a command that starts a tool from `node_modules/.bin`, as the project's dependencies call for),
    nothing under the home directory but the project's own trees, what git reads there (its global config, ignore and attributes files, and the files the config includes or names, never its credential store unless that config names it) and the programs the run pins, `node`'s file among them,
    no network but the run's own loopback,
    for a run that may start a program or that `--allow-temp` grants, a temp directory private to the run that `TMPDIR`, `TMP` and `TEMP` where set, and the XDG data, cache, and state variables name,
    not dumpable, and the `core` limit 0.
    `.env*` in the project stays readable, `/proc` is readable but for the environment, memory, and working directory of a process outside the box.
    Each tool it starts runs in a box of its own inside it, writing a temporary directory of its own, which its `TMPDIR`, its `TMP` and `TEMP` where it has them, and its XDG variables name,
    the command's tool directories for the grants table's tools, and what `--allow-run-write=` names for it, never what `--allow-write=` names;
    no tool reads another's environment, memory, or temporary directory, and the tools see the run's whole environment.
  - `--allow-run-write=<program>:<dirs>` lets one program the command starts write these directories, named as the tool is called (`vitest`), as `--allow-run=` names it, or by its name in the run (`node_modules/.bin/vitest`):
    `gro test --allow-run-write=vitest:src -- -u`, `gro lint --allow-run-write=eslint:src -- eslint --fix`.
    It widens that program's box and the run's, and gives Gro's own code nothing; `--allow-write=` widens no tool's box, so a tool's own write takes this flag.
    A name the command may start no program by, and a plain name both the project's own tool and an `--allow-run=` grant have, are refused before anything runs, exit 2,
    and so is a program's directory under a dot-entry or `node_modules` where Gro's own code writes the whole project (`gen`, `format`, `sync`, or `--allow-write=.`) and no `--allow-write=` names it too.
    `fgro run` takes the flag beside `--allow-run=` naming the program, checks it, and says once that it holds nothing back there.
  - Before the box it refuses `-A` before the command word, or after it for a command the binary runs itself, a `node` on `PATH` that's a script, a write flag or a program's write directory naming a directory that holds the project root,
    a directory any grant flag lists inside the project, relative or absolute, that leads out of it through a symlink, or through one into a dot-entry or `node_modules` it doesn't name, where `fgro run` notes one (so a linked `node_modules` takes the path it leads to, not `--allow-write=node_modules`),
    a `node_modules` symlink to anything but another checkout's `node_modules`, git's files it can't read when the command may start a program,
    a socket on stdio that isn't a connected unix stream, an OS temporary directory inside the project,
    a flag or rule a program's box takes holding the OS temporary directory when the command may start a program (like `--allow-read=/tmp`, `--allow-read=/`, or `--allow-run-write=vitest:/tmp`, never `--allow-write=`, whose directories no program's box takes), and a path planted where the box opens a rule or makes a tool directory,
    each exit 2, and a binary built without its snapshot, exit 1; a step of the box that fails exits 1.
    A `node_modules` linked from another checkout is read there, by the box and Gro's own `fs` alike, and executed for a command that may start a program, its packages resolving within it;
    no tool directory is made there, and a tool writes there only what `--allow-run-write=` names for it.
  - A write the box refuses gains a cause naming where it lets the command write, and a run that fails after a tool did ends with a `note:` line naming the box's network and writes.
  - It needs Linux 6.12 or later (Landlock ABI 6); where Landlock is missing, off, or older, or a seccomp filter refuses its calls, the command runs the project's Node Gro after one `note:` line saying why.

  A command left to Node (like `dev`, `build`, `clean`, an install, a project task, or `task`'s listing),
  or any command in a project that installed another version of Gro, runs the project's Node Gro after one `note:` line saying why;
  `GRO_FALLBACK=command` refuses a fallback the project's files or the host decide.
