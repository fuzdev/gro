---
'@fuzdev/gro': minor
---

**breaking** feat: `should_trigger_gen` takes the changed file's `dependents` set instead of the graph, and `normalize_gen_config` takes the genfile's id

fix: `gro dev --no-watch` runs gen once, in-process through the gen plugin, so it no longer needs a `gro` on `PATH` or in `.bin`; a config without `gro_plugin_gen` no longer gets a gen there, as in watch mode

fix: `gro gen` prints a genfile's `TaskError` as its message, with no stack

fix: a genfile deleted after `gro dev` queued it no longer fails the batch

fix: invalid gen `inputs` are refused naming the genfile, failing it in `gro gen` and logging once per genfile in `gro dev`: `inputs` that aren't `'all'` or an object, `files` or `patterns` that aren't arrays of strings, and a pattern starting `./` or `../` or holding the project root's absolute path (a leading `/` is still the root anchor)

fix: `gro task <dir>` reports an unreadable subdirectory as an error with no stack

fix: `gro` exits `128` plus the signal's number for a task killed by a signal, like a shell
