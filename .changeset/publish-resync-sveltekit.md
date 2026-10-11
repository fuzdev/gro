---
'@fuzdev/gro': patch
---

fix: `gro publish` re-runs `svelte-kit sync` after its post-version-bump install and before `gro gen`, because the install removes the generated `node_modules/$app`, leaving a tsconfig that extends `$app/tsconfig` unresolved while genfiles run - a genfile that analyzes types (like one calling svelte-docinfo's `analyzeFromFiles`) read external types as `any`, or now fails with svelte-docinfo's unresolved-`extends` error
