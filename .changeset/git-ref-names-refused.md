---
'@fuzdev/gro': minor
---

feat: every function in `git.ts` taking a branch or tag name refuses one git would read as more

**breaking** a refspec or pattern like `:deploy`, `a:b`, or `ma*`, a name beginning with `-` or `+` or
`refs/`, `HEAD`, `@`, and anything else `git check-ref-format --branch` refuses throw a `TaskError`
before git runs. `git_fetch` no longer takes a refspec: use `git_fetch_force(git, origin, branch)` for
`+branch:branch`. On a detached `HEAD`, `git_push` and `git_push_to_create` refuse the current branch
and `git_remote_branch_exists` without a branch name throws.
