---
'@fuzdev/gro': patch
---

fix: a task that throws a host module's error (`FsError`, `SpawnError`, `NetError`, `ProcessError`) is reported as failing, with its stack, and no longer as an unexpected error with the hint to reinstall and run `gro clean`
