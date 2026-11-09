---
'@fuzdev/gro': minor
---

fix: dry runs no longer act, and failed commands no longer exit `0`

`gro deploy --dry` keeps the local target branch where it used to delete it when the remote one is
missing. `gro upgrade --dry` no longer pulls, deletes `node_modules` or the lockfile, or runs
`gro sync`, and still passes `--dry-run` to the package manager and refuses paths outside the
project. `gro publish --dry` now refuses a `package.json` with no version or no parseable
`repository`, as a real publish does.

A `TaskError` now fails `gro changeset` for a failed `changeset init`, `changeset add --empty`, the
interactive `changeset`, or `git add`; `gro clean --git` for a failed
`git remote prune`; `gro upgrade` for a failed `gro sync`; and `gro dev --no-watch` for a failed
`gro gen` or genfiles with no `gro` to run them. Watch mode keeps running through a failed gen.

`gro release --force_build` now reaches the build in `gro publish`, which gains `--force_build`.
`gro publish` pushes with `git push --follow-tags --end-of-options <origin> <branch>`, honoring
`--origin` and `--branch`, and its recovery messages name that command.

`gro changeset` writes `--changelog` to the config whichever access is chosen, and logs when it
skips the commit and push because the workspace isn't fully staged. `gro deploy`'s build-failure log
says what happened to the remote branch rather than claiming no git changes. A `throw undefined` or
`throw null` reaching the CLI's uncaught handler now exits `1` rather than crashing it.
