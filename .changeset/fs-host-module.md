---
'@fuzdev/gro': minor
---

feat: add `@fuzdev/gro/fs.ts`, the fs host module, with its Node implementation: directory handles that confine relative paths beneath them (narrowed with `dir`, read-only with `readonly`), a typed `FsError` thrown by default with a no-error path for each op's expected failure, and `fs.project`, `fs.open_dir`, and `fs.temp_dir` (see `src/docs/host_modules.md`)
