---
'@fuzdev/gro': minor
---

**breaking** feat: `load_env` takes a `LoadEnvOptions` object (`{dev, env_dir?, env_files?, ambient_env?}`) instead of positional args, and the `$app/env` shims call it that way; `RenderSveltekitShimAppEnvVarsOptions` extends `LoadEnvOptions`; the `gro` bin and the loader's spawned Node no longer pass `--experimental-strip-types` (unflagged in the supported Node `>=24.14`) or `--disable-warning=ExperimentalWarning`, so experimental warnings from task code now print
