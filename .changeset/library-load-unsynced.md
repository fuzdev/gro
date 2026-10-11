---
'@fuzdev/gro': minor
---

fix: `library_load_from_repo` no longer caches the analysis of a repo whose `tsconfig.json` extends `$app/tsconfig` while `node_modules/$app/tsconfig.json` is missing - its external types analyze as `any`, and the cache kept serving that at the commit after a `gro sync`; it warns and returns the result uncached

- `svelte-docinfo` now logs its diagnostics to a child of the `log` option labelled with the repo dir's name, and its progress at debug level
- added: `sveltekit_tsconfig_is_unsynced` in `sveltekit_helpers.ts` and `SVELTEKIT_TSCONFIG_SPECIFIER` in `constants.ts`
