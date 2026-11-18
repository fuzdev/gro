---
'@fuzdev/gro': minor
---

feat: `create_fs_paths(fs, {cwd, home})` takes the working directory and `$HOME` its `remove_path` and `empty_dir_path` refuse, so `@fuzdev/gro/fs_paths.ts` and `@fuzdev/gro/git.ts` read nothing of the process when they load and import no `node:` module or host module, and load as a package on the runtime

**breaking** `create_fs_paths(fs)` takes a second argument, `FsPathsOptions`: pass `create_fs_paths(fs, {cwd: cwd(), home: env.get('HOME')})`, with `cwd` from `@fuzdev/gro/cwd.ts` and `env` from `@fuzdev/gro/env.ts`, as a project's `gen_task_registry` genfile now does. `cwd` is an absolute real path, as `cwd()` returns, protected as given, and `create_fs_paths` throws `TypeError` for one that isn't absolute; `home` is required and may be `undefined`, which, like an empty or relative one, protects nothing. Before, the boundary read the working directory and `HOME` itself, when it loaded.

**breaking** `RenderSveltekitShimAppEnvVarsOptions` in `@fuzdev/gro/sveltekit_shim_app_env_vars.ts` requires `cwd_module_specifier`, the cwd host module's URL, which the rendered `$app/env/*` modules import to build their `FsPaths`.

fix: `remove_path` and `empty_dir_path` fail before they act when `$HOME`'s real path can't be read for any reason but a missing `$HOME`, where they went ahead protecting only `$HOME` as given. On `fgro`, that read takes `--allow-read` on `$HOME`, or on its nearest existing ancestor when it's missing (a `$HOME` outside every grant is refused, missing or not), and `env.get('HOME')`, which builds the `FsPaths`, takes `--allow-env=HOME`.

`print_command_args` in `@fuzdev/gro/cli.ts` colors through fuz_util's `st`, which `configure_print_colors` sets and the Gro CLI configures unless `NO_COLOR` is set, so it's plain where nothing configured it, where it used Node's `styleText`.
