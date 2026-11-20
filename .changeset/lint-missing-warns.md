---
'@fuzdev/gro': patch
---

fix: `gro lint` warns that nothing was linted when ESLint isn't installed, where it logged the skip at info level, so `gro check` no longer passes its lint step silently; the exit code is unchanged
