---
'@fuzdev/gro': minor
---

fix: self-invalidate the library cache when svelte-docinfo changes

The `.gro/library.json` cache stores svelte-docinfo's module array verbatim,
keyed by the analyzed repo's commit hash — which doesn't move when the
_analyzer's_ svelte-docinfo changes its output shape, so caches written
before the 0.6 `intersects` → `externalTypes` rename kept serving the old
field at an unchanged clean commit. Each record now stamps the installed
svelte-docinfo version and any mismatch is stale. `LIBRARY_CACHE_VERSION`
moves to 2 for the added field, and the `svelte-docinfo` peer range tightens
to `>=0.6.0`.
