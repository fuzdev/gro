---
'@fuzdev/gro': patch
---

fix: a `.css`, `.svg`, or `?raw` import through Gro's loader is exactly the file's text, never evaluated, so a `${…}` in the file no longer runs as JavaScript or fails the import, and its line endings are kept
