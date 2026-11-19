---
'@fuzdev/gro': patch
---

fix: a started child's `stdin.write` after `close()` resolves `false` without dropping the writes before it, where Node's write after the end destroyed the stream and lost what was still queued, the earlier write resolving `false` too
