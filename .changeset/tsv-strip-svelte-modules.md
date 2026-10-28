---
'@fuzdev/gro': patch
---

the loader strips types from `.svelte.ts` modules with tsv's `strip_typescript` instead of `ts-blank-space`, removing the `ts-blank-space` dependency; TypeScript that can't be blanked away (an `enum`, parameter properties, and the like) now fails with the file path and an excerpt
