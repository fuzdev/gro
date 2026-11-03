---
'@fuzdev/gro': minor
---

**breaking** feat: `update_changelog` and `github_fetch_commit_prs` take a required `fetch`, handed to fuz_util's `fetch_value`, which no longer defaults to `globalThis.fetch`
