---
'@fuzdev/gro': minor
---

fix: a builtin refuses its tool's help and version flags in the section it forwards, where the tool printed them in place of running and the step passed, as `gro check -- eslint --help` did

**breaking** `gro lint` refuses `--help`, `-h`, `--version`, and `-v` as whole args in its `-- eslint` section, `gro typecheck` in its `-- svelte-check` and `-- tsc` ones, and `gro build`'s SvelteKit library plugin in `-- svelte-package`; `gro test` (in its `-- vitest` section and its first one, `gro test -- --help`) and the SvelteKit app plugin (in `-- vite`, for `gro dev` and `gro build`) refuse only `--help` and `-h`, as Vitest and Vite ignore a version flag after the command Gro gives them. Each message names the flag, the tool, and running the tool directly. A forwarded `gro` section's `--help`, like `gro check -- gro lint --help`, is refused naming `gro lint --help`, where the task printed its help and passed. Other spellings, and sections only a project task reads, pass through unchanged.
