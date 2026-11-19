---
'@fuzdev/gro': minor
---

feat: a started child's output iterator has `throw`, which drops the rest of the stream, as leaving a `for await` early does, and rejects with what it's given, as the runtime's builtin does
