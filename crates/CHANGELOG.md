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
  It takes the entry file only, with no script arguments.
  Resolution is strict: mandatory extensions, packages only through their `"exports"`,
  `.js` only under `"type": "module"`, and every module kept inside its project's or package's root.
  TypeScript is stripped of its types in process, and TypeScript with runtime semantics, like an `enum`, is refused.
  The global scope holds the ECMAScript globals and a set of web-standard ones (no `fetch` or `crypto`),
  with no `process`, `Deno`, or `Bun`, so code can't read stdin or the environment.
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
