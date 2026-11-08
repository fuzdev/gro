---
'@fuzdev/gro': minor
---

feat: add `@fuzdev/gro/fs.ts`, the fs host module, with its Node implementation: directory handles that confine relative paths beneath them (narrowed with `dir`, read-only with `readonly`), a typed `FsError` thrown by default with a no-error path for each op's expected failure, and `fs.project`, `fs.open_dir`, and `fs.temp_dir` (see `src/docs/host_modules.md`)

feat: add `@fuzdev/gro/fs_search.ts`, a recursive search of a directory through an fs handle, and move Gro off `@fuzdev/fuz_util/fs.ts` onto the fs host module; a permission error, a path through a file, or a symlink loop now throws where Gro used to read it as a missing file, a directory removed while Gro searches it throws instead of the search finding nothing, and a build cache file Gro can't read is left in place rather than deleted

**breaking** feat: `LIBRARY_CACHE_FILENAME` is no longer exported

feat: move the rest of Gro's library and task code off `node:fs` onto the fs host module; an env file whose directory can't be read now throws where Gro skipped it, as an unreadable env file already did, and so does an unreadable ignore file, while a file `gro format` can't read fails the run

feat: `gro format` and the `dist` import rewrite act on regular files only, as tsv's own walk does, skipping a symlink a sweep or glob finds; a path given to `gro format` that names an existing file is that file, its symlinks followed, even with glob syntax in its name like `src/routes/[slug]/+page.ts`, and a glob no longer follows a symlinked directory that its brace alternatives name; `.formatignore` shadows a sibling `.prettierignore`, which is read only without one

feat: `gro deploy` copies a top-level symlink in the build as its target's contents, and the build's inner symlinks as written rather than made absolute

feat: Gro writes a marker, `.gro/.gro_marker`, into a `.gro/` directory it creates, and `gro clean` keeps a `.gro/` symlinked to a directory outside the project that holds the marker and isn't inside a git work tree, like one on a tmpfs, and empties that directory, where it left a symlinked `.gro/` untouched; a `.gro/` linked anywhere else, to an existing directory Gro didn't create included, is unlinked with its target untouched and a warning saying why, as every other symlinked path it cleans is unlinked, so link `.gro/` to a path that doesn't exist yet for Gro to create and mark it; a dangling `.gro/` link is left alone, and Gro creates and marks its target on its next write

**breaking** feat: `gro deploy` refuses, before any git op and with `--dry` too, a deploy dir that resolves to the project, `$HOME`, or an ancestor of either, one inside the project but not beneath `.gro/`, and an existing one that isn't a clone it made (a directory that's its own repo's top level with its own `.git` directory, the project's `origin`, and the local git config `gro.deploy-clone true`, which it now sets in each clone), leaving it in place for the user to remove; a clone without the tag is refused, and removing it lets `gro deploy` clone it again; a failed clone now fails the deploy before its remote is set

**breaking** feat: `gro gen` and `gro gen --check` refuse an output that resolves outside the project or to the project directory itself, its symlinks followed, while a relative `filename` like `'../elsewhere/x.ts'` that stays inside it still works

**breaking** feat: `gro upgrade` refuses a `--node_modules_path` or `--lockfile_path` that resolves outside the project, and `gro clean`, `gro deploy`, and Gro's other removals refuse `$HOME` and its ancestors as they refuse the project's

feat: `gro format` skips a root file like `gro.config.ts` or `vite.config.ts` that's a symlink, as its sweep does

feat: the fs host module refuses a symlink whose target isn't UTF-8, a real path that isn't, and a listing holding a name that isn't, with `io_error`, and a path holding a lone surrogate, with `TypeError`; `fs.temp_dir` throws `TypeError` for a prefix that isn't a string; `FsError` and its prototype are frozen

**breaking** feat: `load_env` and `load_from_env` are async, and `LoadEnvOptions`, `EnvVarDeclarations`, `EnvVisibility`, and `env_var_has_visibility` move to `@fuzdev/gro/env_declarations.ts`

feat: `fs_search` results carry the entry's `kind`

**breaking** feat: `read_format_ignore` resolves to a matcher over absolute ids, reading the ignore file with gitignore semantics through tsv's `IgnoreStack`, so a directory pattern skips everything beneath it and `!` re-includes

**breaking** feat: `BuildOutputEntry` drops `ctime` and its `mode` is the permission bits alone, and `BUILD_CACHE_VERSION` is `'3'`, so an existing build cache misses once

feat: Gro refuses to load in a working directory whose path isn't UTF-8, which Node reports lossily as a different directory

**breaking** feat: `FsError`, `FsErrorKind`, and `is_fs_error` move to `@fuzdev/gro/fs_error.ts`, a module that grants nothing, so code that only makes or matches an fs error doesn't import the host module

feat: add `@fuzdev/gro/fs_paths.ts`, `FsPaths` and `create_fs_paths(fs)`, the boundary from paths that arrive from outside to fs handles (`open_path`, `open_dir_path`, `search_path`, `copy_path`, `empty_dir_path`, `remove_path`), resolving a relative path against `fs.project`, built at a composition root and passed down; its `remove_path` and `empty_dir_path` refuse the project, the working directory Gro loaded in, `$HOME`, and their ancestors, and `FsOpenedPath`, the handle and path it opens, is now public

**breaking** feat: the build cache and `clean_fs` take an `FsPaths` and name their paths relative to its project: `load_build_cache_metadata(fs_paths)`, `validate_build_cache(fs_paths, metadata)`, `collect_build_outputs(fs_paths, build_dirs)`, `discover_build_output_dirs(fs_paths)`, `discover_dist_dirs(fs_paths)`, and `load_build_cache_dependencies(fs_paths)`, with `BUILD_CACHE_METADATA_PATH` the metadata's project-relative path; `save_build_cache_metadata`, `is_build_cache_valid`, and `create_build_cache_metadata` take `BuildCacheDeps` (`{fs_paths, log}`) first and their optional args as `BuildCacheKeyOptions` and `BuildCacheMetadataOptions`, whose `dependencies` is required; `compute_build_cache_key(log, config, options)`; `clean_fs(deps, options)` with `CleanFsDeps` (`{fs_paths, log?}`), `log` moving out of `CleanFsOptions`

**breaking** feat: gen, input paths, the task registry, config loading, and task invocation take an `FsPaths` first, relative paths and default roots now the project's: `find_genfiles(fs_paths, input_paths, root_dirs, config, timings?, root?)`, `analyze_gen_results(fs_paths, gen_results)`, `analyze_gen_result(fs_paths, file)`, `write_gen_results(fs_paths, gen_results, analyzed, log)`, `get_possible_paths(fs_paths, ...)`, `resolve_input_paths(fs_paths, ...)`, `resolve_input_files`' `search` is required, `task_registry_find(fs_paths, dir, search_filters, root?)`, `task_registry_find_builtins(src_lib)` with an `FsReadDir`, `load_gro_config(fs_paths, dir?)`, `resolve_task(fs_paths, task_name, root?)`, and `invoke_cli(fs_paths, task_args, config)`; `InvokeTaskOptions` and `RunTaskOptions` require `fs_paths`, passed to nested tasks but not on `TaskContext`; `gen_task_registry` is a factory, so a genfile exports `gen_task_registry(create_fs_paths(fs))`

fix: a genfile whose result isn't a `RawGenResult`, like a file whose `content` isn't a string (a stale `export const gen = gen_task_registry;`), fails naming the genfile and what it returned, and a genfile whose output fails to format fails rather than writing its outputs unformatted

**breaking** feat: `package_json.ts`, `sveltekit_helpers.ts`, and the install helpers take an `FsPaths` first, relative paths and default directories now the project's: `package_json_load(fs_paths, dir?, cache?, parse?, log?)`, `package_json_sync(fs_paths, map_package_json, log, write?, dir?, exports_dir?)`, `package_json_update(fs_paths, update, dir?, write?)`, `package_json_write(fs_paths, serialized, dir?)`, `has_sveltekit_library(fs_paths, package_json)`, `sveltekit_tsconfig_is_unsynced(fs_paths, dir)`, `sveltekit_sync_if_obviously_needed(fs_paths, sveltekit_cli?)`, `run_svelte_package(fs_paths, options)`, and `gro_package_versions_load(fs_paths, gro_dir?, names?)`; `InstallOrThrowOptions` requires `fs_paths` with `throw_if_gro_packages_changed`; `package_json_load_for_gro()` is sync, reading Gro's `package.json` through the module system; the default config and `gro_plugin_sveltekit_library` build their own, so configs that name a plugin don't change

fix: `package_json_update` and `package_json_sync` write the `package.json` in the `dir` they read, where they wrote the project's
