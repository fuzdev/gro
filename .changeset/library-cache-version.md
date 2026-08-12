---
'@fuzdev/gro': minor
---

fix: self-invalidate the library cache when svelte-docinfo changes

The `.gro/library.json` cache stores svelte-docinfo's module array verbatim,
keyed by the analyzed repo's commit hash — which doesn't move when the
_analyzer's_ svelte-docinfo changes its output shape, so caches written
before the 0.6 `intersects` → `externalTypes` rename kept serving the old
field at an unchanged clean commit. Each record now stamps the installed
svelte-docinfo version in a new `svelte_docinfo_version` field, read from
the same copy `analyzeFromFiles` resolves to and exported as
`SVELTE_DOCINFO_VERSION`; any mismatch is stale.
`LIBRARY_CACHE_VERSION` moves to 2 for the added field, and the
`svelte-docinfo` peer range tightens to `>=0.6.0`, so consumers on 0.4 or
0.5 upgrade with it.
