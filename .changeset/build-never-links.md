---
'@fuzdev/gro': minor
---

feat: `gro build` no longer runs `npm link -f` for a package with `bin`

**breaking** a build never writes outside its project. Run `npm link` once to get a package's CLI on
`PATH`, the link lasts across builds because the plugin still makes the binaries executable.
