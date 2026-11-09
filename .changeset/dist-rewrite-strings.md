---
'@fuzdev/gro': minor
---

**breaking** feat: `rewrite_svelte_ts_imports` is no longer exported

fix: the dist import rewrite leaves template strings and comments as written in `.js` and `.svelte` output, rewriting only the import requests tsv parses, so a generated-code template holding `from './x.ts'` ships intact; `require('./x.ts')` is no longer rewritten and `` import(`./x.ts`) `` now is; a type-position `import('./x.ts')` in a `.svelte` script, erased at compile time, is left as written; a file that doesn't parse falls back to the pattern with a warning; add `rewrite_dist_file_imports`
