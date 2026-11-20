---
'@fuzdev/gro': minor
---

fix: a tool the project declares runs only from its own `node_modules/.bin`, so the tool that runs is the one `package.json` names: ESLint, Vitest, svelte-check, tsc, and `svelte-kit`

**breaking** A declared ESLint, Vitest, svelte-check, or TypeScript missing from the project's `node_modules/.bin` fails with the install hint, where a copy on `PATH` ran. `svelte-kit` for a project that declares `@sveltejs/kit` is found the same way, by `gro typecheck`, `gro sync`, `gro publish`, and the sync before every task, which skips when the project doesn't have it. `to_cli_finder` finds only in the project's `node_modules/.bin`, and `to_local_cli_path` in `cli.ts` names that path. Vite, the changeset CLI, and Gro itself are still found there first and then on `PATH`, through `find_cli`.
