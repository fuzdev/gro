---
'@fuzdev/gro': minor
---

**breaking** refactor: remove unused API: `path_id_to_base_path` from `paths.ts`; the `is_ts` and `ts_transform_options` options of `esbuild_plugin_svelte`, which always transforms `.ts` with `TS_TRANSFORM_OPTIONS_DEFAULT`; `possible_paths_by_input_path` from `resolve_input_paths`'s result; `resolved_input_files_by_root_dir` from `resolve_input_files`'s and `find_genfiles`'s results; and `print_command_args` moves from `args.ts` to `cli.ts`
