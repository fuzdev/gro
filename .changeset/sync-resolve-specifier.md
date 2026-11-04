---
'@fuzdev/gro': minor
---

**breaking** refactor: `resolve_specifier` and `to_sibling_hint` in `resolve_specifier.ts` are synchronous, returning `ResolvedSpecifier` and `SiblingHint | null` instead of promises, and throw where they rejected, with the same errors
