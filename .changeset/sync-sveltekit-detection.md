---
'@fuzdev/gro': minor
---

fix: `gro sync` and `gro typecheck` run `svelte-kit sync` only for a project with SvelteKit in its dependencies

**breaking** `sveltekit_sync_if_available` is removed: gate `sveltekit_sync` on the new `project_has_sveltekit_app`.
A project declaring SvelteKit without it installed now fails `gro typecheck` with the install hint.
The sync before every task reads `package.json` too, so a `svelte-kit` on `PATH` no longer runs in
a project that doesn't declare it. A project only peered on SvelteKit isn't synced by any of them.
