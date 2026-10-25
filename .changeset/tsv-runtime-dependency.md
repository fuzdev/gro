---
'@fuzdev/gro': patch
---

fix: `@fuzdev/tsv` is a runtime dependency again, since `format_file` imports it; 0.221.0 listed it only as a dev dependency, so `gro gen` and `gro format` failed in a project that doesn't install it
