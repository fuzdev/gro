---
'@fuzdev/gro': patch
---

fix: `set_mode` takes `-0` as `0`, which aborted the process on Node 24, whose `chmod` binding asserts an int32 mode
