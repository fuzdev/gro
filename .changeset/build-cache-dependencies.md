---
'@fuzdev/gro': patch
---

fix: the build cache key folds in the Gro version, `process.version`, and a BLAKE3 hash of `node_modules/.package-lock.json` (`null` when missing), so an upgrade or a re-resolved install rebuilds; adds `build_cache_dependencies.ts` and an optional `dependencies` param to `compute_build_cache_key`, `is_build_cache_valid`, and `create_build_cache_metadata`; `BUILD_CACHE_VERSION` moves to `'2'`, so existing caches rebuild once; the docs no longer say secrets in `build_cache_config` are safe, since its unkeyed hash hides only high-entropy values; the `Filer` hashes each file's raw bytes, so binary files that decode alike no longer share a `content_hash`
