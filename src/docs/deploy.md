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
Internally, `gro deploy` builds the target branch in a local clone at `.gro/deploy/`
(`--deploy_dir`), so the target branch is never checked out in your repo;
the cwd checks out and pulls `--source`, and must be clean.
The deploy clone respects its own ignore rules: a `.gitignore` in the build output and your global git excludes.
Files they match are not deployed, and `gro deploy` warns with the list.

`gro deploy` removes and empties the deploy dir, so it checks the dir before anything else, `--dry` included.
A deploy dir inside the project must be strictly beneath `.gro/`, and none may be the project,
your home directory, or an ancestor of either.
An existing deploy dir must be a clone `gro deploy` made: its own repo's top level, not a directory in another repo,
with its own `.git` directory as its git dir, the same `origin` remote as the project,
and the local git config `gro.deploy-clone true`, which `gro deploy` sets in each clone it makes,
so another checkout of the project is never taken for one.
Anything else, a file included, is refused and left in place, for you to remove if nothing in it is needed.
A clone without the tag is refused; remove it and `gro deploy` clones it again.

```bash
gro deploy # prepare build/ and commit it to the `deploy` branch, then push to go live
gro deploy --source my-branch # deploy from `my-branch` instead of the default `main`

# deploy to `custom-deploy-branch` instead of the default `deploy`
# WARNING! this force pushes to the target branch!
gro deploy --target custom-deploy-branch
# the above actually fails because force pushing is destructive, so add `--force` to be extra clear:
gro deploy --target custom-deploy-branch --force
# TODO maybe it should be `--dangerous-target-branch` instead of `--target` and `--force`?

# prepare the deploy dir and push nothing
# (a first deploy still makes a local init commit for the new target branch)
gro deploy --dry

# reset the target branch to its first commit before deploying,
# discarding its history when the final force push publishes it
gro deploy --reset
```

Run `gro deploy --help` or see [`src/lib/deploy.task.ts`](/src/lib/deploy.task.ts) for the details.

For needs more advanced than pushing to a remote branch,
projects can write their own task, like `src/lib/deploy.task.ts`,
run with `gro task src/lib/deploy.task.ts`,
which can call the builtin with `invoke_task('deploy')`.
