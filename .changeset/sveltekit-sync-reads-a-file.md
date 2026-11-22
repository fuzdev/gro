---
'@fuzdev/gro': patch
---

fix: `gen`, `lint`, and `check` run `svelte-kit sync` first when `.svelte-kit/types/route_meta_data.json` is missing, a file the sync writes, where they checked for the `.svelte-kit` directory, so a `.svelte-kit` that exists unsynced, emptied or made by another tool, syncs where it was skipped; `SVELTEKIT_ROUTE_META_DATA_PATH` in `constants.ts` names the file
