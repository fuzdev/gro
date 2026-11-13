---
'@fuzdev/gro': minor
---

fix: a spawn call's `cwd` must be an fs handle from this copy of Gro, checked as `program_in` checks its `dir`

**breaking** a forged object with an absolute `path`, or a handle from another copy of Gro's `fs`, passed as
`cwd` to `run`, `out`, `out_bytes`, or `start` is a `TypeError`, where before the child started in the
directory its `path` named.
