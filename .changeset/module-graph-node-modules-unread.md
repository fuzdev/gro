---
'@fuzdev/gro': patch
---

fix: the module graph never reads a file whose path from the watched directory passes through a `node_modules` directory, so a `#` mapping to a package like `"#dep": "dep"` no longer resolves the package's own imports or logs an error for a missing optional peer
