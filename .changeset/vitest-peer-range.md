---
'@fuzdev/gro': minor
---

feat: `gro test` turns Vitest's module cache off, passing `--fsModuleCache=false` unless a forwarded flag names `fsModuleCache` (like `gro test -- --fsModuleCache`, which turns it back on), since a test could plant code in the cache that a later Vitest run executes; on the binary, where Vitest's box is the grants table's alone, it gives Vitest a module cache in Vitest's own tool directory instead

**breaking** Gro's `vitest` peer and optional dependency ranges are `^5`, dropping vitest 3 and 4, so a project on either upgrades vitest with this Gro.
