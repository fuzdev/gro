---
'@fuzdev/gro': minor
---

**breaking** fix: `library_load_from_repo` throws instead of analyzing a repo whose `tsconfig.json` extends `$app/tsconfig` while `node_modules/$app/tsconfig.json` is missing - its external types analyzed as `any`, and the `.gro/library.json` cache kept serving that at the commit after a `gro sync`; the error names the repo and the fix (`npm install` if needed, then `gro sync` in it), and a cache hit is still served without the check

- `svelte-docinfo` now logs its diagnostics to a child of the `log` option labelled with the repo dir's name, and its progress at debug level
- added: `sveltekit_tsconfig_is_unsynced` in `sveltekit_helpers.ts` and `SVELTEKIT_TSCONFIG_SPECIFIER` in `constants.ts`
