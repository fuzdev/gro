---
'@fuzdev/gro': minor
---

fix: `gro gen` fails for a named directory with no genfiles

**breaking** `gro gen src/foo` used to pass doing nothing, with or without `--check`. The default
scan of `src/` still passes for a project with no genfiles.
