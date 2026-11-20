---
'@fuzdev/gro': minor
---

feat: every project file Gro loads, its tasks, genfiles, and `gro.config.ts`, goes through an `ImportModule` (`modules.ts`) the composition root passes in, the Node roots passing `import_module_node`, `(id) => import(pathToFileURL(id).href)`

**breaking** `InvokeTaskOptions` and `RunTaskOptions` take a required `import_module`, passed to nested tasks as `fs_paths` is. These take it as a new parameter: `load_module(id, import_module, validate?)`, `load_modules(files, validate, map, root, import_module, timings?)`, `load_genfiles(found, root, import_module, timings?)`, `load_task(id, name, root, import_module)`, `load_gro_config(fs_paths, import_module, dir?)`, `invoke_cli(fs_paths, task_args, config, raw_rest_args, import_module)`, `task_registry_find_builtins(src_lib, import_module)`, and `should_trigger_gen(..., root, import_module)`.
