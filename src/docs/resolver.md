# resolver

> How the runtime resolves module specifiers: strict ESM with mandatory extensions,
> packages by `"exports"` only, and the deliberate departures from Node's algorithm

> **status**: built for relative, absolute, and bare specifiers through `"exports"`;
> `"imports"` for `#` subpaths comes later.
> Relative and absolute specifiers aren't bounded by the project root yet;
> a [planned departure](#planned-departures) closes that.

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
  The package is found by self-reference or in `node_modules`, walking up from the importer's real path
  to its project root;
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

- **The `node_modules` search is bounded by the project root, found first:**
  the first `package.json` at or above the importer, outside `node_modules`,
  that isn't a type marker (a JSON object whose keys are all `"type"` or `"sideEffects"`,
  an empty object included, like `dist/esm/package.json`).
  The walk for it runs before any `node_modules` is looked up,
  and the search then runs from the importer up to the root's own `node_modules`,
  so a `node_modules` planted above a project never resolves.
  Any other `package.json` ends the walk, named or not,
  so an uninstalled project fails instead of resolving a parent directory's packages.
  npm and yarn workspaces, whose packages install at the workspace's root, aren't supported.
  Node searches every ancestor's `node_modules`.
- **A file with no project root gets no bare imports except a self-reference,**
  which resolves through its own package's `"exports"` (a module in an installed package importing that package by name).
  With no `package.json` to bound the search, any other bare specifier is refused,
  naming the importer's directory and the fix:
  a `package.json` at the project root, with the package installed in the `node_modules` beside it.
  Its relative imports still resolve.
- **A world-writable directory ends every walk up, and no `package.json` or `node_modules` in it is read:**
  not its `package.json` (as a project root, a `"type"` scope, or a self-reference's scope),
  and not its `node_modules`.
  A package the search finds through one is refused, naming the directory and the fix (remove its other-write bit):
  the `node_modules` it is installed in, its scope's directory (`@scope`), or its own root,
  and, for a linked package (as pnpm's and Deno's stores install them), the real directories holding it,
  or, for a linked `node_modules` or `@scope`, its real parent,
  since any user can plant or replace a package there.
  Module files are outside the rule: one in a world-writable directory still loads
  by a relative specifier or as a package's `"exports"` target, as an entry directly in `/tmp` does,
  though a walk up from it still stops there.
  World-writable is the other-write bit (`0o002`), whatever the sticky bit;
  a group-writable directory doesn't stop a walk.
  The importer's own directory counts,
  so a module directly in `/tmp` runs with relative imports only, and a `.js` file there has no `"type"` scope.
  Node reads them all, so a `package.json` or `node_modules` any user can plant in `/tmp`
  would supply packages, or a self-reference, to every module below it.
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

Decided, not built; it lands before `gro eval`:

- **Origin containment.** A relative or absolute specifier resolves only inside the importer's root:
  the project root (the directory of the `package.json` that bounds the search) for project code,
  the package's real root for package code, and never into a `node_modules` segment or another package's root,
  since each crossing bypasses `"exports"`.
  Node allows all of them.
