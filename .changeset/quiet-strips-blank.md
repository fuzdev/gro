---
'@fuzdev/gro': minor
---

deps: upgrade to `@fuzdev/tsv@0.7.0`; `format_file` still throws on a source that doesn't format, with the same message, now always a plain `Error` (a parse failure was a `SyntaxError` with `start`/`loc`) carrying tsv's error as `cause`; sources tsv 0.7.0 newly rejects (see its changelog) now fail `gro format`
