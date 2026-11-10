---
'@fuzdev/gro': minor
---

fix: `gro check` skips its test step in a project that doesn't declare Vitest

**breaking** only a `dependencies` or `devDependencies` entry for Vitest counts now, so `gro test` in
a project only peered on Vitest throws where it used to run, and its message is now
`no test runner found, add vitest to devDependencies`. `package_json_has_dependency` loses its
`include_peer` param and never counts peer deps. `gro test` reads `package.json` without the schema,
so one the schema refuses no longer fails it.

`gro check` with no Vitest declared, no `-- gro test` section, and no nonempty `-- vitest` one
skips the test step with a log line, or fails naming the test files `gro test` would run when
there are any, so a repo that loses the dependency can't pass without its tests. `gro test` and
`gro check` with Vitest declared but not installed still fail with the install hint.
