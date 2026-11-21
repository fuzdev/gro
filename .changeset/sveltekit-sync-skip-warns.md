---
'@fuzdev/gro': minor
---

fix: the `svelte-kit sync` Gro runs before a task warns when it's skipped because a project declaring `@sveltejs/kit` has a `node_modules` without `svelte-kit` in its `.bin`, where it skipped without a word: `@sveltejs/kit is declared but \`svelte-kit\` isn't in node_modules/.bin, so SvelteKit's files weren't synced - run \`npm i\``; a fresh clone with no `node_modules` still skips quietly, so `gro sync --install` installs first
