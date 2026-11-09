---
'@fuzdev/gro': minor
---

fix: `gro publish` commits the release before building, so its tag lands on the release commit

`changeset publish` tags `HEAD`, and the release commit used to come after it, leaving every tag on
the commit before the version bump. The build and the publish now run on the release commit. A
failed commit stops before npm has the version, and a failed build or `changeset publish` leaves
the release commit local, undone with `git reset --hard HEAD~1`.
