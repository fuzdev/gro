---
'@fuzdev/gro': minor
---

feat: a `BuiltinInvokeGrant` marks an invoke `check: true`, run in check mode, writing no source, so a runtime holding a command to its closure grants the invoked builtin, and the builtins it invokes in turn, without their writes to the project: `check`'s `gen` and `format` are marked, run with `check: true` as before. The mark is data for that runtime; nothing enforces it on Node.
