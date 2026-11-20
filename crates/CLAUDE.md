# crates/

Gro's Rust workspace: the `fgro` runtime, its binary, and the tools that build
and measure it. Gro's root [CLAUDE.md](../CLAUDE.md) §Rust workspace has the
crate map, the commands, and the perf rule; this file holds what an editor of
`crates/` must not miss. Read it before editing anything here.
[vendor/CLAUDE.md](../vendor/CLAUDE.md) holds the vendored crates' rules.

## Crates

`gro_runtime`, `gro_cli`, `gro_sys`, and `gro_bench` are Linux-only: each
crate root holds a `compile_error!` for any other OS, and none has a non-Linux
arm (a `cfg` on the architecture or the C library is fine).

- `gro_runtime` - the JS host: a deno_core `JsRuntime` (deno_core patched,
  in `vendor/`) with `deno_webidl`, `deno_web`, and Gro's bootstrap
  extension; the resolver, the loader, the builtin modules, the host ops
  behind them (`host/`), the engine, the box (`confine/`), and `fgro eval`'s
  protocol (`eval/`). Start at its `lib.rs`
- `gro_cli` - the `fgro` binary (`run`, `eval`, `--version`); its `main.rs`
  maps the commands to their docs
- `gro_sys` - the raw system calls the safe std API lacks (descriptors, the
  environment, `RLIMIT_DATA` and `RLIMIT_NOFILE`, the default thread stack,
  a thread's stack start, `wait4`, a child held by its pidfd and waited on
  by tokio's reactor, in `child`, behind the `child` feature `gro_runtime`
  enables, so `gro_bench` builds no tokio, process groups and the signal
  handler that guards them, `_exit`, `access`, and the filesystem calls
  confined beneath a directory descriptor by `openat2`, in `beneath`), each
  behind a safe API
- `gro_v8` - the V8 boundary: the calls rusty_v8 exposes only as `unsafe`,
  behind a safe API (eval's recording `ArrayBuffer` allocator,
  `record_array_buffers`, and `PendingArrayBuffer`, an `ArrayBuffer`'s bytes
  from the isolate's own allocator, held by Rust alone while the host writes
  them, which the fs reads fill); `v8` is pinned to the exact version
  `deno_core` re-exports through `deno_v8` (its `v8` dependency, a wrapper
  that re-exports `v8`), so a `deno_core` upgrade whose `deno_v8` requires
  another `v8` moves the pin with it
- `gro_bench` - the measurement harness, independent of the runtime crates;
  its `lib.rs` lists its modules and suites
- `xtask` - dev automation (`cargo xtask`): `fetch-v8`, `build`, `check`,
  `check-rust`, `check-js`, `check-reproducible`, `vendor-patch`, `bench`;
  its `main.rs` has the commands and exit codes

## Invariants

They live in the code. Each file named is the authority for its area, in its
module doc; these are the rules an editor of `crates/` must not miss there,
and the contracts no one module doc owns.

- **`unsafe`** only in `gro_sys` and `gro_v8`. Every crate takes the workspace
  lints with `unsafe_code` forbidden; those two re-declare the whole table
  with it at `deny` (Cargo replaces a crate's table whole), and an xtask unit
  test (`crates/xtask/src/lints.rs`) checks each equals the workspace's but
  that and `clippy::undocumented_unsafe_blocks`. Each `unsafe` block is
  allowed by its function's `#[expect]` and explained by a `// SAFETY:`
  comment. `deno_core::scope!` and the `v8` scope macros hide `unsafe` inside
  dependency macros, outside the lint; the runtime uses them for handle scopes.
- The embedded source (`crates/gro_runtime/js/bootstrap.js`) must be 7-bit
  ASCII, or the build fails, and so must the backing scripts' literals
  (`bootstrap/template.rs`). A builtin embeds no module of `src/lib`: what it
  shares with its Node module is behavior, held by the differential cases,
  never source.
- The resolver (`resolver/`) is a pure module: no deno_core types, reading
  only through the `SourceTree` trait. The loader owns the admitted roots
  (`Roots`) and passes them into `resolve_from`. Failures are enums (the
  resolver's `ResolveErrorKind`, the loader's `FailureReason`), never parsed
  from a message; eval's error kind comes from the step that failed and the
  graph's record.
- The builtin table (`resolver/builtin.rs`) is the one source of the builtin
  modules: the bootstrap names each row's backing script and synthetic module
  from it, and no snapshot holds a backing script, which `deno_core` runs at
  a run's first import of the row (`bootstrap.rs`'s module doc).
  - A backing script (`bootstrap/template.rs`) only asks the host for its
    namespace: it names no global but `undefined`, reads no member of an
    intrinsic, and decides nothing; whatever has identity or state is the
    host's, in `OpState`. Its test holds every row's script to the two texts
    in the module doc, byte for byte.
  - Every row's members are native (`host/native.rs`): made from the table
    `native_members!` declares, the first time a run asks for the row's
    namespace through the one op, `op_gro_namespace`, which knows the row by
    the script that calls it. No host op is a `deno_core` async op: an
    `async` member returns a promise the host settled before returning it,
    and a `later` member one the host settles in a later turn, counted
    while pending so the event loop waits (`host/later.rs`'s module doc has
    the rules).
  - A first `import()` of a builtin short of stack is built from the event
    loop (`loader/headroom.rs`), and both levels of a backing script stay safe
    to end at any call and to run again (`bootstrap.rs`'s module doc has the
    rules). An op a level calls must be safe to call again, and nothing checks
    that.
  - `crates/gro_cli/tests/host.rs` imports every row in a realm of trapped
    intrinsics and exercises everything it reaches with hostile arguments and
    receivers (its module doc; `tests/fixtures/host/patched/exercise.js`). A
    function it reaches fails until `patched/use.js` has an entry for it, and
    a use of a hostile argument until `ARGUMENT_USES` lists it for that
    function, reviewed.
  - An error row's class and guard are the host's (`host/error_class.rs`): a
    member throws only a family it declares (`throws FAMILY(name)` in
    `native_members!`), since throwing one and making its class each take a
    `DeclaredFamily`, which only the macro makes.
  - A handle is the host's (`host/handle.rs`): a frozen object of a prototype
    the host made, its state under its kind's brand, a private symbol. A
    method refuses anything without the brand and decides from the handle's
    state, never from the run's grants, which gate only that the run holds
    the row. A handle's native Rust state (a `blake3` hasher, a `tsv` ignore
    stack) lives in a `NativeTable` of the run's, which drops it once the
    handle is collected.
  - An `fs` handle's state is plain data, and only a root holds a descriptor
    (`host/fs.rs`'s module doc maps its children and owns the `openat2`
    lookups beneath a handle; `host/fs/roots.rs`'s owns the run table of
    roots, the view whose scopes join within it alone (the project's, a
    run's one view), their nesting, and the rule that keeps a root's
    directory where it is, `host/fs/walk.rs`'s the walk of an absolute
    path from a view's roots,
    `host/fs/reads.rs`'s and `host/fs/listing.rs`'s the reads and
    listings, and `host/fs/writes.rs`'s the writes): what a handle's op may
    do is the handle's scope, never the run's roots, which only that rule
    reads, to refuse, and a refusal's hint, to name what would let it
    through; and no lookup leaves a root. An op's I/O takes owned
    arguments, so it can move off the thread, but for a read's fill of the
    buffer V8 allocated (`gro_v8::PendingArrayBuffer`, not `Send`), which
    stays on the isolate's thread.
  - The resolver returns a table hit ahead of any package lookup; the loader
    checks it against the run's `Authority` at link time, for a static import
    and `import()` alike (`loader/resolve.rs`): with none (eval's) nothing is
    imported, a grant-free row included, checked before the grant; a
    confined run's (`Authority::Confined`) package module is granted none,
    and a grant-free row links for the project and a package alike; an
    unconfined run (`Authority::Unconfined`, `fgro run -A`) grants every
    module every row, and lifts no refusal but a grant's. A builtin keeps
    no per-importer state.
  - A builtin declares no types: its types are its Node module's.
    `src/test/builtin_types.test.ts` holds the native members' table, as
    `tests/native_members.txt` pins it, to each row's Node module: names,
    kinds, and arity. `crates/gro_cli/tests/host.rs` holds every row's values
    to what its Node module printed (`tests/fixtures/host/expected.json`,
    which `node oracle.js --write` there regenerates under Node).
  - The table also reserves specifiers it doesn't serve
    (`resolver/builtin.rs`), refused ahead of any package lookup; a package's
    bare import into the project root is refused
    (`ResolveErrorKind::PackageImportsProject`); the loader's refusals of a
    string-compiled `import()` and of attributes, and their order, are
    `loader/mod.rs`'s.
- Type state: `confine::Prepared` → `Prepared::confine` → `Confined`, or
  `Confinement::Unconfined` named by `fgro run`, `make-snapshot`, and unit
  tests; `engine::init_engine` takes a `Confinement` and returns an `Engine`,
  which `run_main` and `make_snapshot` require, and `init_confined_engine`
  takes a `Confined` and returns the `ConfinedEngine` `eval::eval` requires.
  No runtime starts before the process is confined or named unconfined; a
  confined engine whose JIT disagrees with the box's is
  `EngineError::JitConflict`.
- One thread until the box: `fgro eval` keeps its process single-threaded
  until it's confined, in the step order written once, in `confine_eval`'s
  doc in `crates/gro_cli/src/eval.rs`. Every other command runs its runtime on
  a thread of its own, not the main one, sized `gro_runtime::STACK_SIZE`.
- `bootstrap::extensions` drops every lazy ES module and every other
  extension's synthetic ones, since `deno_core` would load either past the
  loader's refusal (its doc has how), so a synthetic module's name must be
  one the resolver refuses from source.
- One grant choke point: the one host op, `op_gro_namespace`, is declared
  through `host::host_ops!`, whose first step is `require_authority`
  (`host/mod.rs`). Nothing at compile time makes a host op use the macro: a
  test fails on an op of gro's extension that's neither a host op nor on
  `bootstrap::UNGATED_OPS`.
- The same choke point for native members: each is declared through
  `native_members!`, whose callback checks the grant first and hands the
  body each argument unconverted, no native function is made outside
  `native.rs` and `error_class.rs`, and no promise resolver outside
  `native.rs` and `later.rs`; source-scanning tests hold them
  (`host/native.rs`'s module doc lists them). `tests/native_members.txt`
  pins the table as `op_names.txt` pins the ops.
- `bootstrap::REFUSED_OPS` lists the ops eval refuses. Of `deno_web`'s ops
  and classes the runtime registers only those on `bootstrap::WEB_OPS` and
  `WEB_CLASSES`, which `bootstrap/web_scripts.rs` holds to the scripts.
  `crates/gro_runtime/tests/op_names.txt`, `dropped_op_names.txt`, and
  `refused_op_names.txt` pin them, so a `deno_core`/`deno_web` upgrade that
  adds an op fails until it's reviewed (`vendor/README.md`'s upgrade steps).
- A start from the snapshot binds no op again (`skip_op_registration`), so
  nothing may call `op_load_ext_script` or `op_lazy_load_esm` once a run has
  started, and the runtime's tests start from a snapshot too, in child
  processes, as `cargo xtask build --verify` runs them; `snapshot.rs`'s
  module doc (§A start from the snapshot) has the rule and the tests.
- No snapshot is made of ops eval's refusals can't land on:
  `snapshot::make_snapshot` fails when a refused op's function shares its
  address with another op's (`snapshot.rs`'s module doc has why).
- `gro_runtime` reads no environment variable but the `phases` feature's
  `GRO_PHASES_FILE` and the `prices` feature's `GRO_PRICES_OFF` (`lib.rs`'s
  module doc, which also names the vendored `deno_core`'s own diagnostic
  reads, which only eval clears). What a run takes from the process, `fgro`
  reads once as the run starts and hands in through `RuntimeOptions`, as
  `gro_cli/src/run.rs`'s module doc lists (§What a run is handed). The
  `phases` and `prices` features (on `gro_cli`, enabling `gro_runtime`'s) are
  off in the binary that ships; without them nothing reads the variables.
- A run's or an eval's runtime is handed back when it's over
  (`SpentRuntime`), and every command exits through `end` in
  `crates/gro_cli/src/main.rs` but for eval's out-of-memory backstop. A host
  module that buffers stdout or stderr must have it written before the
  runtime is handed back and before `exit` calls the embedder's step
  (`lib.rs`'s module doc).
- `gro_runtime` ends its embedder's process itself in two places only
  (`lib.rs`'s module doc), beside the signal handler a run that grants a
  program installs at its first launch, which ends the process by the
  signal; such a run needs the process made ready first
  (`RuntimeOptions::spawn_env`). A program's `exit` calls the step the embedder
  gave (`RuntimeOptions::exit_step`; `host/exit.rs` has why), and a run that
  grants `exit` without one fails before it starts. `fgro`'s step is
  `run::exit_now`, and `run::exit_code` is the one place a run's exit code
  is decided. A refusal by rule, or for a grant a handle lacks, fails the
  run caught or not, recorded, with the grant that would have allowed it
  where one would (`loader/refused.rs`), from which `fgro` prints its hints. No in-process
  test grants a step that returns.
- A change to the seccomp allowlist reruns the census
  ([eval](../src/docs/eval.md#changing-the-allowlist)).

## Tests

Confined eval runs only from a snapshot build, so
`crates/gro_cli/tests/eval/`'s tests that evaluate are ignored in a plain
`cargo test` and run from `cargo xtask build --verify`, beside
`tests/confine.rs` (no libtest harness, whose threads would refuse the box).
They need Landlock ABI 6 and fail below it, naming the kernel's ABI.
`--verify` names each of `gro_cli`'s integration tests with the features it
runs under (`INTEGRATION_TESTS` in `crates/xtask/src/snapshot.rs`), and an
xtask unit test fails on a file of `crates/gro_cli/tests/` no run names.
`gro_sys`'s tests that close descriptors, clear the environment, cap memory,
or signal run in a child process, harness off.

## Reproducible builds

A clean checkout of one commit, the same toolchain (the system's C compiler
and library included), and the same cargo home build the same `fgro`, byte for
byte, wherever the checkout is; `fgro` embeds its commit, so a dirty tree
builds other bytes. `cargo xtask check-reproducible` holds it for the release
build; `crates/xtask/src/reproducible.rs` says what it refuses and what it
can't see.

## Benchmarks

`gro_bench` times runtimes × workloads in fresh processes, interleaved, with
an A/A noise row, and appends one JSON line per run to
`crates/gro_bench/history.jsonl`; its `lib.rs` describes the suites, and
`crates/xtask/src/bench/mod.rs` the flags. A change to a workload project's
committed files bumps its suite's `version` and re-pins its `blake3`. Record a
suite on a quiet machine, after `cargo xtask build --release`:
`cargo xtask bench crates/gro_bench/suites/baseline/suite.json --attest-quiet`
(or `suites/eval/suite.json`).
