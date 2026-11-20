---
'@fuzdev/gro': patch
---

fix: the loader loads a `type: 'json'` import of a package's file, like `svelte-docinfo/package.json` or a package importing its own JSON, which it refused with `ERR_IMPORT_ATTRIBUTE_MISSING`; `library_load.ts` and `package_json_load_for_gro` import JSON this way, so they need this loader or none
