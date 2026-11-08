---
'@fuzdev/gro': minor
---

feat: add `@fuzdev/gro/fs.ts`, the fs host module, with its Node implementation: directory handles that confine relative paths beneath them (narrowed with `dir`, read-only with `readonly`), a typed `FsError` thrown by default with a no-error path for each op's expected failure, and `fs.project`, `fs.open_dir`, and `fs.temp_dir` (see `src/docs/host_modules.md`)

feat: add `@fuzdev/gro/fs_search.ts`, a recursive search of a directory through an fs handle, and move Gro off `@fuzdev/fuz_util/fs.ts` onto the fs host module; a permission error, a path through a file, or a symlink loop now throws where Gro used to read it as a missing file, a directory removed while Gro searches it throws instead of the search finding nothing, and a build cache file Gro can't read is left in place rather than deleted

**breaking** feat: `LIBRARY_CACHE_FILENAME` is no longer exported
