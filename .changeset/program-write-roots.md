---
'@fuzdev/gro': minor
---

feat: a program's grant in a builtin's `task.grants` takes `write`, the tool directories it writes on its own (`BuiltinToolDir` in `@fuzdev/gro/builtin_host.ts`), which the binary's box holds the program to and nothing reads on Node; `svelte-kit` names `.svelte-kit`, `node_modules/$app`, and `node_modules/.vite-temp`, svelte-check `.svelte-kit/generated/dev` and `node_modules/.vite-temp`, and Vitest those, `.svelte-kit/types`, and `node_modules/.vite/vitest`, and every builtin that syncs SvelteKit grants it by `SVELTEKIT_CLI_GRANT` from `@fuzdev/gro/sveltekit_helpers.ts`

`gro test` now runs `svelte-kit sync` before Vitest in a SvelteKit project, as `gro typecheck` does, so Vitest finds SvelteKit's files in `node_modules/$app` current and writes nothing there; `gro test --no-sync` skips it, and `gro check` passes it to `test` once its typecheck has synced.

**breaking** `run_test`'s `TestDeps.programs` finds `sveltekit_cli` beside `vitest_cli`, and its `Args` gain `sync`, which a caller passing args by hand sets, since left out no sync runs.
