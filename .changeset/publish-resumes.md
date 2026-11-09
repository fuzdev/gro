---
'@fuzdev/gro': minor
---

feat: `gro publish` resumes an untagged release commit, and `--optional` works on Changesets 3

A release commit with no tag is a publish that stopped at the build or `changeset publish`, and
rerunning `gro publish` now picks up from it. Gro reads `.changeset` itself to decide whether there
is anything to release, since `changeset version` exits `1` with no changesets on Changesets 3,
which failed `gro publish --optional` and `gro release` for a package with nothing to publish.
`gro publish` with no changesets now says so, `--dry` included.
