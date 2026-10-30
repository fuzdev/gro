// The module-graph workload: loads every module of fuz_util's `src/lib` whose
// runtime imports are all relative, transitively (no bare or `node:`
// specifier), from the corpora snapshot the suite pins, as TS each runtime
// strips itself. Changing the set, or re-pinning the corpora, is a suite
// version bump.
//
// It runs inside the collection's own root, the way a project imports its own
// files: the suite names the `fuz_util` collection, so the bench writes a
// fresh copy of it from the pinned tree (under the gro checkout's gitignored
// `target/bench/`) and runs this file from the copy's root, beside fuz_util's
// `package.json`. Its imports are relative to that root, so none leaves it, as
// a runtime confining relative imports to the importer's root requires. Every
// command still runs from the suite's directory.
//
// Its cold row turns off each runtime's on-disk caches of transpile output
// and compiled code, so nothing is read from them or written to them: deno's
// emit and V8 code caches, and bun's transpiler cache (deno's
// dependency-analysis cache has no switch and stays on). Node has neither by
// default (its compile cache is opt-in), so the `node` cold and warm rows run
// the same command. Cold times the uncached load, not the cache writes of a
// first run. The warm row keeps each runtime's defaults, filled by its warmup
// processes.
//
// The `node_compile_cache` runtime is node with its compile cache on
// (`NODE_COMPILE_CACHE`), in the gitignored `target/` of the gro checkout,
// resolved from the suite's directory, where every command runs; its cold row
// turns the cache off (`NODE_DISABLE_COMPILE_CACHE`).
//
// The `fgro` runtime is gro's own, the release build with its startup
// snapshot that `cargo xtask build --release` makes; `cargo xtask bench`
// builds it before a run that names it. It strips this graph's TS in process
// and has no on-disk cache of any kind, so, like `node`, its cold and warm
// rows run the same command.

import './src/lib/array.ts';
import './src/lib/async.ts';
import './src/lib/benchmark.ts';
import './src/lib/benchmark_format.ts';
import './src/lib/benchmark_stats.ts';
import './src/lib/benchmark_types.ts';
import './src/lib/bytes.ts';
import './src/lib/colors.ts';
import './src/lib/counter.ts';
import './src/lib/dag.ts';
import './src/lib/deep_equal.ts';
import './src/lib/diff.ts';
import './src/lib/dom.ts';
import './src/lib/error.ts';
import './src/lib/fact_store.ts';
import './src/lib/fractional_index.ts';
import './src/lib/function.ts';
import './src/lib/hash.ts';
import './src/lib/hex.ts';
import './src/lib/iterator.ts';
import './src/lib/json.ts';
import './src/lib/library_json.ts';
import './src/lib/lru_map.ts';
import './src/lib/map.ts';
import './src/lib/maths.ts';
import './src/lib/object.ts';
import './src/lib/package_helpers.ts';
import './src/lib/path.ts';
import './src/lib/pkg_json.ts';
import './src/lib/print.ts';
import './src/lib/random.ts';
import './src/lib/random_alea.ts';
import './src/lib/random_xoshiro.ts';
import './src/lib/regexp.ts';
import './src/lib/result.ts';
import './src/lib/sort.ts';
import './src/lib/stats.ts';
import './src/lib/string.ts';
import './src/lib/svelte_preprocess_helpers.ts';
import './src/lib/throttle.ts';
import './src/lib/time.ts';
import './src/lib/timings.ts';
import './src/lib/types.ts';
import './src/lib/zod.ts';
