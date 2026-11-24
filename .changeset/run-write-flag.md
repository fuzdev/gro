---
'@fuzdev/gro': minor
---

feat: the CLI takes `--allow-run-write=<program>:<dirs>` among the binary's grant flags, split off by `to_grant_flags` like the rest and, like them, warned of and confining nothing on the Node gro; `gro run` given it with no `--allow-run` is a usage error, and a bare `--allow-run-write` names its own example, `--allow-run-write=vitest:src`.
