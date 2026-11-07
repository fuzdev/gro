---
'@fuzdev/gro': minor
---

**breaking** feat: `run_svelte_package`, `sveltekit_sync`, and `format_directory` take an options object (`RunSveltePackageOptions`, `SveltekitSyncOptions`, `FormatDirectoryOptions`) instead of positional args; `run_svelte_package`'s `options` and `cli` are named `svelte_package_options` and `svelte_package_cli`, like `gro_plugin_sveltekit_library`'s options
