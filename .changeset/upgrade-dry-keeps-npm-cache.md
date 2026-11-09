---
'@fuzdev/gro': minor
---

fix: a dry `gro upgrade` no longer runs `npm cache clean --force` on an ETARGET failure

`install_with_cache_healing` takes `heal_cache`, `true` by default, and `gro upgrade` passes `false`
when dry, so the failure surfaces and nothing outside the dry install runs.
