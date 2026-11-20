---
'@fuzdev/gro': minor
---

feat: the library modules the CLI loads, but for the exceptions `src/test/package_module_closure.test.ts` lists, read paths as `/`-separated text, through `internal/posix_path.ts`, and color through fuz_util's `st`, importing no `node:path`, `node:util`, `node:url`, or `node:module`

**breaking** `package_json_load_for_gro` is async, importing Gro's `package.json` as JSON when first called. The modules that moved to fuz_util's `st` print uncolored until something calls `configure_print_colors`, as the CLI does. `to_search_filters_relative` takes an absolute `root` and `search_dir`, and every project root a path function takes is absolute, a relative one a `TypeError` where it read against the working directory: `to_task_path`, `to_input_path`, and the `task_args.ts` functions that take a `root` among them.
