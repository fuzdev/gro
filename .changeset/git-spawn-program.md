---
'@fuzdev/gro': minor
---

add `git.ts`, git over a `SpawnProgram`, and move every git call in Gro onto it

`git.ts` carries `GitOrigin`, `GitBranch`, and the `git_*` functions Gro uses, each taking the `git`
program as its first argument and a `cwd` handle in its options, so `@fuzdev/fuz_util/git.ts` is no
longer used here. A root builds the program, with `GIT_LOCAL_PASS_ENV` for a read of the local repo
and `pass_env: 'all'` where git commits or reaches a remote.

`library_load_from_repo` and `library_cache_key` take a `LibraryLoadDeps` of `{fs_paths, git}` in
place of `fs_paths`. `BuildCacheDeps` gains `git`, and `compute_build_cache_key` takes
`{git, log}` in place of `log`; `save_build_cache_metadata` takes the new narrower `BuildCacheSaveDeps`.

A name passed to git now follows `--end-of-options`, and `git_pull` throws for an origin or branch
beginning with `-`, closing a path where `--upload-pack=...` given as `--origin` ran that command.
A local read stopped before it finished now throws instead of returning what it had read.
Local reads capture with `out` under a cleared child environment.

`git_current_commit_hash` takes no branch and reads `HEAD` through `rev-parse`, so a detached
`HEAD` has a commit, a branch named like another can't shadow it, and a remote's `HEAD` no longer
keys the cache. `gro build` in a directory that isn't a repo now builds, warning that the cache
isn't reused, where it used to fail reading the branch.

A failed `git status` no longer reads as clean: `gro build`, `gro deploy`, `gro publish`, and
`gro check --workspace` throw on a repo git can't read, like a corrupt index or dubious ownership.
A directory that isn't a repo still reports no changes, and the library cache key is `null` for an
unreadable repo, so its analysis runs fresh. A local read whose output was truncated now throws.

`gro commit` and `gro changeset` stop with a `TaskError` when `git commit` fails rather than
pushing anyway. A task that needs git and can't find it says so as a `TaskError`, and
`gro check --workspace` passes with a warning.
