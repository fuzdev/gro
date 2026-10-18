# deploy

The [`gro deploy`](/src/lib/deploy.task.ts)
task was originally designed to support static deployments to
[GitHub pages](https://pages.github.com/),
but what it actually does is just [build](./build.md) and push to a branch.

> Where it's going: on the [runtime](runtime.md), `deploy` and `publish` are the named unconfined phases,
> shown that way in the policy diff, with credentials passed only to the process that needs them.

Importantly, Gro **destructively force pushes** to the `--target` branch, `deploy` by default.
This is because Gro treats your deployment
branch as disposable, able to be deleted or squashed or whatever whenever.
Internally, `gro deploy` uses [git worktree](https://git-scm.com/docs/git-worktree)
for tidiness.

```bash
gro deploy # prepare build/ and commit it to the `deploy` branch, then push to go live
gro deploy --source my-branch # deploy from `my-branch` instead of the default `main`

# deploy to `custom-deploy-branch` instead of the default `deploy`
# WARNING! this force pushes to the target branch!
gro deploy --target custom-deploy-branch
# the above actually fails because force pushing is destructive, so add `--force` to be extra clear:
gro deploy --target custom-deploy-branch --force
# TODO maybe it should be `--dangerous-target-branch` instead of `--target` and `--force`?

gro deploy --dry # prepare build/ but don't commit or push
gro deploy --clean # if something goes wrong, use this to reset git and gro state
```

Run `gro deploy --help` or see [`src/lib/deploy.task.ts`](/src/lib/deploy.task.ts) for the details.

For needs more advanced than pushing to a remote branch,
projects can implement a custom `src/lib/deploy.task.ts`.
