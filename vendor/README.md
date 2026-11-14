# Vendored crates

Patched copies of published crates. Each `vendor/<name>/` is the crate as
published, extracted unmodified from its checksum-verified `.crate`, plus
gro's patch, kept beside it as `vendor/<name>.patch`: the whole difference,
files gro adds included. The root `Cargo.toml` puts each in place of the
registry crate with `[patch.crates-io]` and excludes it from the workspace, so
gro's lints, formatting, and checks don't reach it (ESLint ignores `vendor/`).
Each patched site carries a one-line `gro patch:` comment in its file's
comment syntax (`// gro patch:` in Rust).

These checks hold the tree to this index:

- `cargo xtask check-rust` (and `check`) fetches each crate's `.crate` from
  crates.io once, caching it under `target/vendor/<sha256>/`, verifies the
  SHA-256 recorded below, applies the patch, and fails unless the result
  equals the vendored tree, contents and executable bits, naming each file
  that differs
  (`crates/xtask/src/vendor.rs`)
- its unit tests check that every file a patch touches is listed below, that
  every file it modifies carries a marker, and that each tree's markers are
  the ones its patch adds

The patch is regenerated from the tree, never edited by hand:

```sh
cargo xtask vendor-patch deno_core
```

It writes `vendor/deno_core.patch` as `git diff` does, with `--- a/<path>`
and `+++ b/<path>` headers relative to the crate's root, the vendored tree
against the extracted `.crate`.

## deno_core

- upstream: `deno_core` 0.412.0, the `.crate` published on crates.io, SHA-256
  `580cb2e0d797d75ff3dfc4a8f2b14aa7470f0fa31654a09a83d4cbb42b5eb9b4` (the
  checksum crates.io's index records for it)
- patch: [deno_core.patch](deno_core.patch)
- `LICENSE`, added: the MIT text the crate's file headers name, which the
  published crate doesn't ship

The patch makes a V8 startup snapshot a function of its inputs: the same
sources make the same blob, byte for byte, in every process and checkout. It
also turns off source maps, makes one internal key public for gro's
prepare-stack-trace callback, refuses a source-phase import of a synthetic
module, and hands the loader an `import()`'s attributes (the sections after
the snapshot's).
`cargo xtask build --verify` checks it, remaking the snapshot with the built
`fgro` and comparing.

### Hash order in the snapshot

Snapshot serialization wrote several `HashMap`s and `HashSet`s in iteration
order, which is random per process, so the blob changed every run. Each site
now sorts first.

- `modules/module_map_data.rs`, `ModuleNameTypeMap::drain`: walks the submaps
  in creation order and each submap's entries by name
- `modules/module_map_data.rs`, `ModuleMapData::serialize_for_snapshotting`:
  sorts `lazy_esm_specifiers` by name, and sorts `loaded_script_results` by
  name before registering them with V8, whose registration order also orders
  V8's part of the blob
- `runtime/snapshot.rs`, `SnapshottedData::encode`: writes `ext_source_maps`
  sorted by key

Reportable upstream: yes, as a reproducible-snapshot fix.

### A freed buffer copied into the snapshot

`JsRuntime::store_js_callbacks` hands JS `ArrayBuffer`s over `ContextState`'s
`tick_info`, `immediate_info`, and `timer_info`, with a no-op deleter since
`ContextState` owns the bytes. `JsRuntimeForSnapshot::snapshot` destroys the
realm, freeing `ContextState`, before V8 creates the blob, so V8 read freed
memory and copied whatever then occupied it into the blob, a use-after-free
read.

- `runtime/jsrealm.rs`, `ContextState`: a new field, `shared_info_buffers`,
  holding those buffers
- `runtime/jsruntime.rs`, `JsRuntime::store_js_callbacks`: keeps each buffer
  in `shared_info_buffers`
- `runtime/jsrealm.rs`, `JsRealmInner::destroy`: detaches the buffers before
  the state is freed

A detached buffer serializes empty, and a runtime started from the snapshot
hands JS fresh buffers as before. In ordinary teardown no JS runs after
`destroy`, so the detach changes nothing but closing the dangling views.

Reportable upstream: yes, as a memory-safety bug.

### No source maps

fgro applies no source maps, as Node doesn't by default: a position names the
code that ran (TypeScript needs none, since the type strip keeps every line and
column). `deno_core` maps locations through a module's inline map wherever it
reads one: a `JsError`'s frames, the call sites a program's
`Error.prepareStackTrace` reads, and a failed import's position.

- `source_map.rs`, `SourceMapper::apply_source_map`: returns `Unchanged`
  first, the one place every mapping goes through
- `source_map.rs`, `SourceMapper::add_source_map_url`: keeps nothing, so a
  bundle's inline map isn't retained for each module that carries one

`ext_source_maps` stays as it is: no extension gro links has one, and the
snapshot carries the same bytes.

Reportable upstream: no, a policy of gro's.

### The call sites' key, public

gro's prepare-stack-trace callback keeps an error's call sites where
`deno_core`'s `JsError` reads its frames from, under `deno_core`'s private
symbol.

- `error.rs`, `call_site_evals_key`: `pub`, was `pub(crate)`

Reportable upstream: yes, for embedders that set their own callback.

### A source-phase import of a synthetic module

A synthetic module (an extension's `synthetic_esm_modules` facade, which
`deno_core` builds from its backing script's exports) has no source, and its
load skipped the check that refuses a source-phase import of a JavaScript
module. `import.source` of one then reached the `expect` that reads the
module's source and panicked, aborting the process.

- `modules/recursive_load.rs`, `RecursiveModuleLoad::register_and_recurse_impl`:
  a source-phase request for a synthetic module is refused with a
  `ReferenceError`, as one for a JavaScript module is, before the module is
  built

Reportable upstream: yes, as a panic reachable from JS.

### An `import()`'s attributes, passed to the loader

`deno_core` parses an `import()`'s attributes, then resolves the call with
none, so a loader never sees them and can't refuse one: gro refuses every
attribute under a key but `type`, and any attribute of a builtin module, as
it does a static import's. A static import's reach the loader already.

- `modules/map/dynamic.rs`, `ModuleMap::load_dynamic_import`: takes the
  call's attributes and resolves with them
- `runtime/bindings.rs`, `host_import_module_with_phase_dynamically_callback`:
  passes the attributes it parsed

Reportable upstream: yes, so a loader sees the attributes of both forms of
import.

## Upgrading a vendored crate

1. Bump the version in the root `Cargo.toml` (for `deno_core`, with `deno_web`
   and `deno_webidl`, and the `v8` pin to the version it re-exports). With the
   old copy still patched in, cargo warns the patch is unused and resolves the
   registry crate, downloading and verifying the new `.crate` against the
   index's checksum.
2. Record the new `.crate`'s SHA-256 above (`sha256sum` of
   `~/.cargo/registry/cache/*/<name>-<version>.crate`, which equals the
   `checksum` cargo wrote to `Cargo.lock`).
3. Replace `vendor/<name>/` with that `.crate` extracted (a gzipped tarball),
   then reapply the patch in it (`patch -p1 -i ../<name>.patch`), adapting
   each site, dropping any part upstream has fixed, and updating the entries
   above.
4. `cargo xtask vendor-patch <name>` regenerates the patch from the tree.
5. `cargo update -p <name> --offline` points `Cargo.lock` back at the path,
   then `cargo xtask check` reruns the vendor check and, in its snapshot
   build, the reproducibility gate.
