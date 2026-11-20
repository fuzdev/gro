---
'@fuzdev/gro': minor
---

feat: loading the library modules the CLI reaches reads nothing of the process: the project root is a parameter, the CLI's from the cwd host module, and the CLI's listeners on the global `process` live in a Node-only internal module

**breaking** `paths.ts` no longer exports `paths`, `infer_paths`, or `IS_THIS_GRO`: build a project's `Paths` with `create_paths(cwd())` from the cwd host module, or `create_paths(fs_paths.project.path)`. `gro_paths` is always Gro's own package, found from its module URL. These take the absolute project root as a new parameter: `print_path(path, root)` and `to_root_path(id, root)` (a path in Gro's package still prints against Gro's root), `create_empty_gro_config(root)`, `cook_gro_config(raw, root)`, `load_task(id, name, root)`, `load_modules(files, validate, map, root, timings?)`, `load_genfiles(found, root, timings?)`, `run_gen(modules, config, log, timings, invoke_task, root, format_file?)`, `to_gen_result(origin_id, raw, root)`, `create_gen_context(base, origin_id, root)`, `should_trigger_gen(..., reported_invalid_inputs, root)`, and `normalize_gen_config` and `gen_inputs_match`, whose `root` had defaulted to the cwd.
