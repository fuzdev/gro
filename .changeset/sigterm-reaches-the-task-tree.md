---
'@fuzdev/gro': minor
---

fix: a `SIGTERM` sent to `gro` alone now reaches the whole task tree

The spawn host module sends a host's `SIGTERM` on to children with an inherited stream, so each
layer of the CLI passes it down instead of leaving the rest running. `SIGINT` and `SIGHUP` still
come from the terminal.
