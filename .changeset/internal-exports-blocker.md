---
'@fuzdev/gro': minor
---

feat: block `internal/` directories from generated package exports

Each `internal/` directory under `src/lib` (any depth) gets a null exports
entry from `gro sync` — `"./internal/*": null`, `"./domain/internal/*":
null` — Node's explicit-exclusion form, best-matching the directory's
subpaths ahead of the broader wildcards. Internal modules ship in dist for
public modules to import but can't be imported by consumers, and internal
files no longer count toward which wildcard export patterns are emitted.
`svelte-docinfo` honors the same signal in exports discovery and excludes
`internal/` directories from analysis at any depth by default.
