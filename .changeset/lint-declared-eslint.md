---
'@fuzdev/gro': minor
---

feat: `gro lint` runs ESLint only when `package.json` declares it, as `gro test` does Vitest, and `gro check` decides its lint step by the declaration, as it does its test step

**breaking** `gro lint` fails with `no linter found, add eslint to devDependencies` when ESLint isn't in `dependencies` or `devDependencies`, so an undeclared one on `PATH` no longer runs; declared but not installed in the project's `node_modules/.bin`, it fails with the install hint `gro test` gives. `gro check` without ESLint declared skips its lint step with a log line, unless the project's root has an ESLint config (`eslint.config.*` or a legacy `.eslintrc*`), and then fails naming it: add eslint to devDependencies, remove the config, or pass `--no-lint`. A `-- gro lint` or nonempty `-- eslint` section forwarded to `gro check` runs the lint step, which decides. `project_has_vitest` in `vitest_helpers.ts` is replaced by `project_has_dependency(fs_paths, dep_name, log)` in `package_json.ts`, and `lint`'s grants read the project for `package.json`.
