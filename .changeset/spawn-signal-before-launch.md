---
'@fuzdev/gro': patch
---

fix: `spawn` listens for the host's signals before it launches a child, so a signal that reaches the host while a child starts is handled as it is once the child runs (a `SIGTERM` sent on to an inheriting child, the group of a group-leading one killed), where the signal ended the host and left the child running; a launch that fails keeps the listeners and the event loop for two more turns, so a signal that came during it still ends the host
