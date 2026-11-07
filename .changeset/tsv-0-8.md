---
'@fuzdev/gro': minor
---

deps: upgrade to `@fuzdev/tsv@0.8.1` and drop `oxc-parser`; `gro format` and `format_file` take its formatting fixes, so `gro format --check` can flag files formatted by 0.7, and sources tsv 0.8 newly rejects (see its changelog) now fail `gro format` and the loader's `.svelte.ts` type strip. The Filer's import graph reads imports with tsv's `parse_imports_typescript` / `parse_imports_svelte`: `import x = require()` is an edge, `export {type X} from` is skipped as type-only, and a file that doesn't parse (a Svelte component's markup included) warns with its location and keeps the edges it last parsed with, where oxc recovered a partial list
