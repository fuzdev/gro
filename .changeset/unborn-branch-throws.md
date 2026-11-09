---
'@fuzdev/gro': minor
---

feat: `git_current_branch_name` throws on an unborn branch

**breaking** it used to return the literal `HEAD`, which read as a branch name. The `TaskError` names
the case and the fix, and it now reaches what reads the branch first: `git_checkout` throws from an
unborn branch instead of switching away from it, so `gro deploy` and `gro publish` fail from an unborn
branch, as do `git_push`, `git_push_to_create`, and `git_remote_branch_exists` with no branch. A detached `HEAD` still returns `HEAD`. `gro deploy` still replaces a deploy clone left on
an unborn branch, and `gro commit` still makes a repo's first commit.
