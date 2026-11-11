---
'@fuzdev/gro': minor
---

feat: add `read_link` to the fs host module's `FsReadDir`, and `set_times`, `set_mode`, and `symlink` to `FsDir`

`symlink(path, target)` takes the path first and stores any target verbatim, returning `'created'` or
`'exists'`; a link's target is data, and following it stays confined. `set_times` takes milliseconds,
`set_mode` an integer from `0` to `0o777`, each else a `TypeError`.

**breaking** `FsReadDir` and `FsDir` grow these ops, so a hand-written implementation or fake of either
no longer typechecks until it has them.

**breaking** `write`'s `mode` is masked to `0o777`, so a sticky bit it used to set on a new file is
dropped, and a mode that isn't an integer from `0` to `0o7777`, like the string `'755'` Node parsed as
octal, is a `TypeError`.

`gro_plugin_sveltekit_library` makes a package's bins executable with `set_mode` rather than spawning
`chmod +x`, adding the execute bits for everyone whatever the umask and dropping a setuid, setgid, or
sticky bit.
