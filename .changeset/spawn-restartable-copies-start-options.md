---
'@fuzdev/gro': minor
---

fix: `spawn_restartable` copies its `args` and `options` at creation

A caller changing its array, options object, or `stdio` tuple afterward no longer changes what a
restart runs. `cwd` and `signal` stay the same handle and signal, so aborting still stops a child.
