---
'@fuzdev/gro': minor
---

fix: `gro deploy --dry` keeps an existing local target branch, and `--origin` works on a fresh clone

A dry deploy clones the local target branch as it is, where it used to overwrite it from the remote.
The deploy clone now names its remote after `--origin`, so `git remote set-url` finds it.
