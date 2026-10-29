# resolver

> How the runtime resolves module specifiers: strict ESM with mandatory extensions,
> packages by `"exports"` only, and the deliberate departures from Node's algorithm

> **status**: built for relative, absolute, and bare specifiers through `"exports"`;
> `"imports"` for `#` subpaths comes later.
> Relative and absolute specifiers aren't bounded by the project root yet,
> and a file with no `package.json` above it searches every ancestor's `node_modules`;
> the [planned departures](#planned-departures) close both.

The runtime resolves modules with its own strict algorithm, not Node's.
Bridging on the legacy algorithm would let code work on gro by accident of legacy resolution
and break when the bridge is removed, and the strict subset is small enough that a bridge saves little.
It's written once in `gro_runtime` as a pure module with no engine types:
it reads only through a source-tree trait and returns the resolved URL,
the module's origin (project or package), and the `package.json` files it consulted,
so other tools can reuse it and a test can drive it without V8.
`import.meta.resolve` goes through it too, so it needs the file to exist.

## Specifiers

- **Relative and absolute** specifiers resolve as URLs with mandatory extensions:
  no probing, no directory index, no `main`.
- **Bare** specifiers resolve through a package's `"exports"` with a fixed condition set
  (`gro`, `import`, `default`, matched in the package's own key order, Node's algorithm),
  and `"imports"` for `#` subpaths (not built yet).
  The package is found by self-reference or in `node_modules`, walking up from the importer's real path;
  the importer's origin is passed in, since a linked package's real path has no `node_modules` in it.
- **Files on disk only.** No `https:`, `jsr:`, or `npm:` imports,
  and gro never pre-bundles a project's dependencies.
- **Refused, with the reason and a fix where there is one:** a package without `"exports"` (no `main`),
  a CommonJS package or file, a `.js` file whose nearest `package.json` lacks `"type": "module"`,
  schemes other than `file:`, and the file types [typescript](typescript.md#file-types) lists
  (`.cts`, `.tsx`, declaration files, and Svelte modules).
  The fix for a package is a version with an ESM build,
  an ESM bundle of it vendored into the project, or running it on Node.

## Departures from Node

Otherwise the resolver follows Node's algorithm.
Where it departs, it departs on purpose, and always toward failing closed:

- **The `node_modules` walk ends at the project root:** the first `package.json` outside `node_modules`
  that isn't a type marker (a JSON object whose keys are all `"type"` or `"sideEffects"`,
  an empty object included, like `dist/esm/package.json`).
  Any other one ends the walk, named or not,
  so an uninstalled project fails instead of resolving a parent directory's packages.
  npm and yarn workspaces, whose packages install at the workspace's root, aren't supported.
- **Every `package.json` the walk passes outside `node_modules` is read,** to tell a marker from a bound,
  so one that isn't valid (not a JSON object, a `"name"` or `"type"` that isn't a string,
  over 16 MiB, or a special file) fails the import, where Node never reads it.
- **A duplicated key keeps its last value,** as `JSON.parse` does, where Node checks every occurrence.
- **A `.js` file with no `package.json` above it is refused,** where Node guesses its format from the code
  (`.mjs` needs none).
- **`.ts` inside `node_modules` is stripped like any other,** where Node refuses.
- **A `.ts` file is always an ES module,** where Node reads `"type"` and the code and may run it as CommonJS;
  `.cts` is refused.
- **A scoped name with an empty, `.`, or `..` segment is refused,** where Node accepts it.
- **No other condition matches:** not `node`, `require`, `browser`,
  or `svelte` until the Svelte compiler is in.
- **A pattern's match can't hide a `..` behind a tab or a line break** (`.\t.`), which URL parsing drops,
  where Node substitutes it unchecked and loads what it names outside the package.

## Planned departures

Decided, not built; they land before `gro eval`:

- **Origin containment.** A relative or absolute specifier resolves only inside the importer's root:
  the project root (the directory of the `package.json` that bounds the search) for project code,
  the package's real root for package code, and never into a `node_modules` segment or another package's root,
  since each crossing bypasses `"exports"`.
  Node allows all of them.
- **The bound is found first, and a file with no project gets no bare imports.**
  The search for the bounding `package.json` runs before any `node_modules` is consulted,
  so a `node_modules` planted above a project-less file never resolves,
  and a file with no `package.json` above it is refused the bare import with the fix.
- **A world-writable directory ends every walk**, and nothing in it is read:
  no `package.json`, no `node_modules`, so `/tmp` can't supply a package to a file below it.
