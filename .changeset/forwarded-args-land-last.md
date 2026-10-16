---
'@fuzdev/gro': patch
---

fix: `invoke_task` merges forwarded args with the new `merge_forwarded_args` of `args.ts`, which puts each forwarded key after the direct args, so a forwarded flag wins a `no-x` pair (the pair resolves this way with `@fuzdev/fuz_util` >=0.75.1): `gro x --color -C -- gro x --color`, with `C` an alias of `no-color`, gives `color` where the forwarded `--color` lost to the direct `-C`; a forwarded key's value overrides the direct one as before; `run_svelte_package` serializes its forwarded `svelte-package` args the same way, after the configured options, so a forwarded flag is the last typed
