---
'@fuzdev/gro': patch
---

fix: `stdout.write` and `stderr.write` called on anything but their handle reject with a `TypeError`, where they threw it, so every failure of a write is a rejection; a view of a detached buffer is refused with the module's own message, and a fixed-length view a resize left out of its buffer's bounds writes as empty, where it was refused, as on `fgro`
