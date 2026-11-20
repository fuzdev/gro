---
'@fuzdev/gro': minor
---

fix: `gro check --workspace` fails when git isn't found on `PATH` or the directory isn't in a git repo, where it warned or passed silently, as a workspace check that was asked for and can't run doesn't pass

**breaking** `git_check_workspace` resolves to `null` for a directory that isn't in a git repo, where it resolved to a clean status, so a caller decides what no repo means; `git_check_clean_workspace` and `git_check_fully_staged_workspace` still read no repo as clean. `git_parse_workspace_status` takes the output as a `string`, no longer `null`. A dirty workspace fails with `--workspace checks the git workspace, and git has …`, naming removing the untracked files as a fix when there are any, where the message named `git_check_clean_workspace`.
