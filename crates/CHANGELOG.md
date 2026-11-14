# fgro changelog

> `fgro`, gro's experimental JS runtime binary, has its own version line,
> released from `fgro-v<version>` tags apart from the npm package's;
> each section here becomes its GitHub Release's notes,
> and is written ahead of its tag, so the tags say what has shipped

0.1.0 isn't tagged yet.

## 0.1.0

The first release of `fgro`, a Rust binary on V8 using Deno's web-standard libraries,
for Linux x86_64 only.
It ships as a bare binary, `fgro-linux-x64-gnu`, with its startup snapshot embedded,
beside a `SHA512SUMS` file, each with a provenance attestation.

- `fgro run <file>` runs a JS or TypeScript module from disk.
  Every argument after the file is the script's, passed as written,
  and the script reads them as `args` from `@fuzdev/gro/args.ts`,
  the first builtin host module.
  `fgro run` grants it to the project's own modules: a package's module is refused it when the graph links,
  and can't import the project's own modules by name to reach it through one,
  and `fgro eval` imports no builtin module.
  Three more builtin modules need no grant, so a package's modules import them as the project's do:
  `@fuzdev/gro/cwd.ts`, whose `cwd()` is the directory `fgro` was started in,
  `@fuzdev/gro/pid.ts`, whose `pid` is the process's id,
  and `@fuzdev/gro/runtime_version.ts`, whose `runtime_version` is `fgro/<version>`.
  Each has the exports and types of its Node module in the npm package.
  The `.js` spelling `@fuzdev/gro/args.js`, and the host modules not on `fgro` yet (`@fuzdev/gro/fs.ts`, `@fuzdev/gro/env.ts`, and the rest),
  are refused, as is a respelling of these or of `@fuzdev/gro/args.ts` (`@fuzdev/gro/f%73.ts`, `@fuzdev/gro/arg%73.ts`), never loaded from the npm package.
  So are `@fuzdev/gro/process.ts` and `@fuzdev/gro/process_error.ts`, which Gro doesn't have, either spelling,
  the refusal naming the modules to import instead.
  An `import()` in code compiled from a string (`eval`, a `Function` constructor, a string timer) is refused,
  since V8 can share that code between the modules compiling the same string, so it has no importer whose grants could be checked.
  Import attributes follow the language's rule: an attribute under any key but `type` is refused before anything resolves,
  for a static import and `import()` alike, and an import of a builtin module with any attribute is refused, `import()`'s included.
  An `import()` refused by one of gro's rules (an ungranted builtin module, a reserved specifier, a refused scheme, and the like)
  is reported on stderr after the run and its error even when the program catches it, once per specifier, importer, and reason, escaped and bounded,
  the first refusal naming each host module by the project and by each of the first 32 packages listed past any flood, and one naming a host module listed even when it's the run's error, the exit code unchanged; one refused because nothing is there, like a missing optional dependency, or because a feature isn't supported, isn't.
  Resolution is strict: mandatory extensions, packages only through their `"exports"`,
  `.js` only under `"type": "module"`, and every module kept inside its project's or package's root.
  TypeScript is stripped of its types in process, and TypeScript with runtime semantics, like an `enum`, is refused.
  The global scope holds the ECMAScript globals and a set of web-standard ones (no `fetch` or `crypto`),
  with no `process` global, `Deno`, or `Bun`, so code can't read stdin or the environment.
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
