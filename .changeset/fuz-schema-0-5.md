---
'@fuzdev/gro': minor
---

**breaking** chore: peer '@fuzdev/fuz_schema' `^0.5.0`, which caps a parse at a hundred issues (a `SchemaError` lists the first ten and `and N more`), labels every issue beneath a `message`, and refuses a `pipe` action narrower than the step before it
