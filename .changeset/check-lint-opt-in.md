---
'@fuzdev/gro': minor
---

feat: make `gro check`'s lint step opt-in with `--lint`

**breaking** `gro check` no longer lints by default: pass `--lint` to run the lint step, as CI's `gro check --workspace --build --lint` should, and `gro publish` now passes it to its check. The `no-lint` arg is gone from `check`'s `Args` (`--no-lint` still parses, as `lint: false`), so a wrapper that invoked `check` with `'no-lint': true` passes nothing or `lint: false`. Args forwarded to the linter, a `-- gro lint` section or a nonempty `-- eslint` one, fail without `--lint` rather than being dropped, and a plain `check`'s command closure no longer holds `lint` or ESLint's grants.
