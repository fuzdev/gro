---
'@fuzdev/gro': minor
---

fix: `watch.dir`'s `dir` must be an fs handle from this copy of Gro, checked as a spawn call's `cwd` is

**breaking** a forged object with an absolute `path` and a `list`, or a handle from another copy of Gro's
`fs`, passed to `watch.dir` is a `TypeError` naming `dir`, where before the watch was on the directory its
`path` named.
