---
'@fuzdev/gro': minor
---

fix: fs and spawn handles, their roots, `FsPaths` and its opened paths, and `spawn_restartable`'s result are null-prototype, so awaiting one adopts no polluted `Object.prototype.then`

**breaking** these objects no longer inherit from `Object.prototype`, so coercing one to a string
(`${handle}`, `String(handle)`) is a `TypeError` where it gave `[object Object]`, `handle instanceof Object`
is false, and `hasOwnProperty` and `toString` are gone from them. An opened path no longer strictly equals a
plain object literal with the same fields.
