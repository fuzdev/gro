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
The one exception is a single `;`, `(`, or `)` written into blanked space
where removing a type would change how the code parses (an ASI hazard), which keeps every position.
Syntax with runtime semantics is refused with its position and a fix:

- `enum` (a `declare enum` erases)
- a namespace with runtime code (a types-only or `declare` namespace erases)
- parameter properties (`constructor(private x: number)`)
- `import =`, `import type x =`, `export import`, and `export =`
- angle-bracket assertions (`<T>x`)
- an unparenthesized `as` or `satisfies` whose removal would regroup the expression
  (`a + b as T / c`, which Node's stripper silently reads as `a + b / c`)
  or leave it invalid (`-x as T ** 2`)

Some shapes tsc rejects but that carry a value (`declare x = 1`, `abstract x = 1`, `declare let a = 1`)
are blanked whole, so the value is dropped silently, as ts-blank-space does.

Imports follow `verbatimModuleSyntax`.
`import {type T}` keeps the import, so its side effects run,
while `import type` and `export type … from` drop entirely.
An `import {T}` naming only a type is kept as written,
so V8 fails it at link time with a `SyntaxError`; mark it `type`.

Every file parses as a module, with no retry as a script,
and a parse error reads `cannot parse the TypeScript: …` with its position.

## File types

- `.ts` and `.mts` load as ES modules, always: `"type"` isn't read for them,
  where Node lets a `.ts` file be CommonJS.
- `.ts` inside `node_modules` is stripped like any other file, where Node refuses it.
- Refused with the reason: `.cts` (CommonJS), `.tsx` (JSX), declaration files,
  and Svelte modules (`.svelte.ts`, `.svelte.test.ts`, any name with `.svelte.` ending in `.js` or `.ts`),
  which need the Svelte compiler.
