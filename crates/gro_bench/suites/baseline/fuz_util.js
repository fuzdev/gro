// The module-graph workload: loads every module of fuz_util's `src/lib` whose
// runtime imports are all relative, transitively (no bare or `node:`
// specifier), from the corpora snapshot the suite pins, as TS each runtime
// strips itself. Changing the set, or re-pinning the corpora, is a suite
// version bump.
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

import '../../../../../corpora/collections/fuz_util/src/lib/array.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/async.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/benchmark.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/benchmark_format.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/benchmark_stats.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/benchmark_types.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/bytes.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/colors.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/counter.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/dag.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/deep_equal.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/diff.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/dom.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/error.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/fact_store.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/fractional_index.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/function.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/hash.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/hex.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/iterator.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/json.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/library_json.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/lru_map.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/map.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/maths.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/object.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/package_helpers.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/path.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/pkg_json.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/print.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/random.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/random_alea.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/random_xoshiro.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/regexp.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/result.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/sort.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/stats.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/string.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/svelte_preprocess_helpers.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/throttle.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/time.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/timings.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/types.ts';
import '../../../../../corpora/collections/fuz_util/src/lib/zod.ts';
