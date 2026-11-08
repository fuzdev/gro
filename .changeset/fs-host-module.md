---
'@fuzdev/gro': minor
---

feat: add `@fuzdev/gro/fs.ts`, the fs host module, with its Node implementation: directory handles that confine relative paths beneath them (narrowed with `dir`, read-only with `readonly`), a typed `FsError` thrown by default with a no-error path for each op's expected failure, and `fs.project`, `fs.open_dir`, and `fs.temp_dir` (see `src/docs/host_modules.md`)

feat: add `@fuzdev/gro/fs_search.ts`, a recursive search of a directory through an fs handle, and move Gro off `@fuzdev/fuz_util/fs.ts` onto the fs host module; a permission error, a path through a file, or a symlink loop now throws where Gro used to read it as a missing file, a directory removed while Gro searches it throws instead of the search finding nothing, and a build cache file Gro can't read is left in place rather than deleted

**breaking** feat: `LIBRARY_CACHE_FILENAME` is no longer exported

feat: move the rest of Gro's library and task code off `node:fs` onto the fs host module; an env file whose directory can't be read now throws where Gro skipped it, as an unreadable env file already did, and so does an unreadable ignore file, while a file `gro format` can't read fails the run

feat: `gro format` and the `dist` import rewrite act on regular files only, as tsv's own walk does, skipping a symlink a sweep or glob finds; a path given to `gro format` that names an existing file is that file, its symlinks followed, even with glob syntax in its name like `src/routes/[slug]/+page.ts`, and a glob no longer follows a symlinked directory that its brace alternatives name; `.formatignore` shadows a sibling `.prettierignore`, which is read only without one

feat: `gro deploy` copies a top-level symlink in the build as its target's contents, and the build's inner symlinks as written rather than made absolute; `gro clean` keeps a `.gro/` symlinked to a directory outside the project, like one on a tmpfs, and empties that directory, where it left a symlinked `.gro/` untouched, while a `.gro/` linked anywhere else is unlinked, as every other symlinked path it cleans is; `gro deploy` refuses a deploy dir that resolves to the project or one of its ancestors

**breaking** feat: `load_env` and `load_from_env` are async, and `LoadEnvOptions`, `EnvVarDeclarations`, `EnvVisibility`, and `env_var_has_visibility` move to `@fuzdev/gro/env_declarations.ts`

feat: `fs_search` results carry the entry's `kind`

**breaking** feat: `read_format_ignore` resolves to a matcher over absolute ids, reading the ignore file with gitignore semantics through tsv's `IgnoreStack`, so a directory pattern skips everything beneath it and `!` re-includes

**breaking** feat: `BuildOutputEntry` drops `ctime` and its `mode` is the permission bits alone, and `BUILD_CACHE_VERSION` is `'3'`, so an existing build cache misses once
