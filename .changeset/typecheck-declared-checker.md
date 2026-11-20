---
'@fuzdev/gro': minor
---

feat: `gro typecheck` runs svelte-check or tsc only when `package.json` declares it, as `gro test` does Vitest, with the project's Svelte files as the evidence it needs svelte-check

**breaking** svelte-check runs when it's in `dependencies` or `devDependencies`, where it ran whenever it was found, so an undeclared one on `PATH` no longer runs. With it declared, a nonempty forwarded `-- tsc` section fails, as tsc doesn't run. Without it declared, a nonempty forwarded `-- svelte-check` section, which asks for it, fails, and a project whose `src/` has `.svelte` files fails naming them: add svelte-check to devDependencies. Otherwise tsc runs when `typescript` is declared, and with neither declared `gro typecheck` fails with `no typechecker found, add typescript to devDependencies`, where it failed only when neither was found. A declared checker not installed in the project's `node_modules/.bin` fails with the install hint `gro test` gives. `find_svelte_files` in the new `svelte_helpers.ts` finds the files.
