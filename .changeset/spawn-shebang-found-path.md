---
'@fuzdev/gro': minor
---

fix: a `#!` script spawned through a symlink runs at the path it was found at, so its `$0` is the name it was called through

A script that dispatches on `$0`, like Debian's `psql` and `pg_isready` (links to the `pg_wrapper`
Perl script), failed or exited 0 with no output, as it saw the pinned real path. A native binary
still runs at its pinned real path with the granted name as `argv[0]`.

**breaking** a script is pinned by identity, its device and inode: at each launch its found path
must still lead to the same file, or the call throws `SpawnError` of kind `launch_failed` where it
used to run the file at the pinned path. That refuses a link retargeted or removed and a file
renamed over the script, linked or not, while the same file reached by a new real path still runs.
A script behind a `node_modules/.bin` link sees the `.bin` path as `$0` (a Node script's
`process.argv[1]`), as from a shell.
