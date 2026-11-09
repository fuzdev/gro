---
'@fuzdev/gro': minor
---

feat: add `SpawnProgram.out_bytes` and `SpawnOutOptions.stdin`, capturing bytes and keeping the terminal for commands that may prompt

**breaking** `SpawnProgram` gains a required `out_bytes`, so a hand-written `SpawnProgram` needs one.
