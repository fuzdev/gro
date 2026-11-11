---
'@fuzdev/gro': minor
---

feat: add `read_link` to the fs host module's `FsReadDir`, and `set_times`, `set_mode`, and `symlink` to `FsDir`

`symlink(path, target)` takes the path first and stores any target verbatim, returning `'created'` or
`'exists'`; a link's target is data, and following it stays confined. `set_times` takes milliseconds,
`set_mode` an integer from `0` to `0o777`, each else a `TypeError`.

**breaking** `FsReadDir` and `FsDir` grow these ops, so a hand-written implementation or fake of either
no longer typechecks until it has them.
