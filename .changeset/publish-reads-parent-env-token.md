---
'@fuzdev/gro': minor
---

feat: `gro publish` reads `SECRET_GITHUB_API_TOKEN` from the parent directory's `.env` too

One token in a workspace's `.env` serves its repos again, for this key only. `load_from_env` still
reads only the project's `.env` by default.
