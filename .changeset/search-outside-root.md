---
'@fuzdev/gro': patch
---

fix: a search of a directory outside the project root tests `search_filters` with paths relative to that directory, so `gro task ~/.cache/other/src`, `gro gen` of such a directory, and `gro format` of it or a glob into it find files under a `.`-prefixed directory instead of nothing; `to_search_filters_relative` takes an optional `search_dir`
