---
'@fuzdev/gro': minor
---

**breaking** fix: `load_from_env` no longer reads `../.env` by default (`paths` defaults to `['.env']`), so `gro publish` takes `SECRET_GITHUB_API_TOKEN` only from the env or the project's `.env`; and `gro deploy` stages the build with `git add .` instead of `git add . -f`, so files the deploy clone's ignore rules match (a `.gitignore` in the build output, the user's global excludes) are no longer deployed, and `gro deploy` warns with the list
