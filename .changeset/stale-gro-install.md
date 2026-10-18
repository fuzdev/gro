---
'@fuzdev/gro': patch
---

fix: `gro sync --install` (and so `gro build`, `gro deploy`, and `gro publish`) and `gro changeset` throw a `TaskError` asking to re-run when their install changed the installed version of Gro or its dependencies, instead of continuing with a mix of old and new modules
