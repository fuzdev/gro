---
'@fuzdev/gro': patch
---

fix: Gro runs when its install path holds URL or JS string syntax, like `#`, `%20`, or `"`, since `spawn_with_loader_args` encodes its `data:` URL and registers the loader by its file URL
