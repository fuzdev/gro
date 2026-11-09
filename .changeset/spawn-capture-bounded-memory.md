---
'@fuzdev/gro': minor
---

fix: `SpawnProgram.out` and `out_bytes` hold each stream in one array, so many small writes cost only their bytes
