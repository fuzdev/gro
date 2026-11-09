---
'@fuzdev/gro': minor
---

fix: `gro build` verifies only builds that started clean, and `--allow_dirty` no longer caches a changed tree

The post-build check compared a coarse status, so a dirty start hid most changes while still failing
on some. It now applies only to a clean start. A clean start that changes under `--allow_dirty`
passes without saving its build cache, which used to record that tree under the commit.
