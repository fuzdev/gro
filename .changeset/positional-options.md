---
'@fuzdev/gro': minor
---

**breaking** feat: `run_svelte_package`, `sveltekit_sync`, `to_define_import_meta_env`, `load_sveltekit_env_declarations`, and `format_directory` take an options object (`RunSveltePackageOptions`, `SveltekitSyncOptions`, `ToDefineImportMetaEnvOptions`, `LoadSveltekitEnvDeclarationsOptions`, `FormatDirectoryOptions`) instead of positional args; `run_svelte_package`'s `options` and `cli` are named `svelte_package_options` and `svelte_package_cli`, like `gro_plugin_sveltekit_library`'s options
