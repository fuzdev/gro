# typescript

> How the runtime runs TypeScript: types stripped in process by tsv with every position preserved,
> erasable syntax only, and what's refused

> **status**: built; `fgro run` loads `.ts` and `.mts` this way.

The runtime runs TypeScript by type stripping only.
[tsv](https://github.com/fuzdev/tsv) blanks the types in process,
so every position in the output matches the source
and stack traces point at the TS file with no source maps.
There is no transpile step: the stripped text goes straight to V8.

## Erasable syntax only

Like Node's stripper, gro erases types and never transforms code.
Syntax with runtime semantics is refused with its position and a fix:

- `enum`
- a namespace with runtime code (a types-only or `declare` namespace erases)
- parameter properties (`constructor(private x: number)`)
- `import =` and `export =`
- angle-bracket assertions (`<T>x`)
- an unparenthesized `as` whose removal would regroup the expression (`-x as number ** 2`)

Imports follow `verbatimModuleSyntax`:
`import {type T}` keeps the import, so its side effects run,
while `import type` and `export type … from` drop entirely.

## File types

- `.ts` and `.mts` load as ES modules.
- `.ts` inside `node_modules` is stripped like any other file, where Node refuses it.
- Refused with the reason: `.cts` (CommonJS), `.tsx` (JSX), declaration files,
  and Svelte modules (`.svelte.ts`, `.svelte.test.ts`, any name with `.svelte.` ending in `.js` or `.ts`),
  which need the Svelte compiler.
