# resolver

> How the runtime resolves module specifiers: strict ESM with mandatory extensions,
> packages by `"exports"` only, and the deliberate departures from Node's algorithm

> **status**: built for relative, absolute, and bare specifiers through `"exports"`,
> with every module contained in its project's or package's root, and for the builtin modules;
> `"imports"` for `#` subpaths comes later.

The runtime resolves modules with its own strict algorithm, not Node's.
Bridging on the legacy algorithm would let code work on Gro by accident of legacy resolution
and break when the bridge is removed, and the strict subset is small enough that a bridge saves little.
It's written once in `gro_runtime` as a pure module with no engine types:
it reads only through a source-tree trait and returns a builtin module or a module on disk:
the resolved URL, the file's identity,
the root the module is inside and its origin (project or package), and the `package.json` files it consulted,
so a test can drive it without V8, and other tools could reuse it once its roots can be built outside the crate.
The module loader owns the roots a graph has admitted and passes them in;
the resolver admits nothing.
`import.meta.resolve` goes through it too, so it needs the file to exist.

## Roots and origins

A module's origin is a function of its real path over the graph's admitted roots:
the project root, and the real root of every package a bare specifier resolved into.
The innermost root above the path wins, by path component,
and a path with a `node_modules` segment below that root is in none, so it's never the project's.
The project root is the entry's: the directory of the `package.json` that bounds its search,
or the entry's own directory when it has none.
An entry inside `node_modules` runs as the package it's in, the one after the last `node_modules` segment,
with the project root at the directory holding the first;
one in no package (`node_modules/.bin/x.mjs`, `node_modules/loose.mjs`) has no origin and is refused.
Roots are keyed by real directory,
so a package directory reached under two names keeps its first admission's origin.
A linked package's real path has no `node_modules` in it, so only its admission says it's a package.

## Specifiers

- **Relative and absolute** specifiers resolve as URLs with mandatory extensions:
  no probing, no directory index, no `main`.
- **Bare** specifiers resolve through a package's `"exports"` with a fixed condition set
  (`gro`, `import`, `default`, matched in the package's own key order, Node's algorithm),
  and `"imports"` for `#` subpaths (not built yet).
  The package is found by self-reference or in `node_modules`, walking up from the importer's real path
  to its project root.
- **Files on disk only,** but for the builtin modules. No `https:`, `jsr:`, or `npm:` imports,
  and Gro never pre-bundles a project's dependencies.
- **Refused, with the reason and a fix where there is one:** a package without `"exports"` (no `main`),
  a CommonJS package or file, a `.js` file whose nearest `package.json` lacks `"type": "module"`,
  schemes other than `file:` (the host's own `ext:` modules and the builtin modules' `builtin:` URLs included), and the file types [typescript](typescript.md#file-types) lists
  (`.cts`, `.tsx`, declaration files, and Svelte modules).
  The fix for a package is a version with an ESM build,
  an ESM bundle of it vendored into the project, or running it on Node.

## Builtin modules

A bare specifier in the builtin table names a [builtin module](glossary.md#builtin-module), a host module the binary implements:
today `@fuzdev/gro/process.ts` alone ([host modules](host_modules.md#process-on-fgro)).
The table is checked before any package lookup, matching the specifier exactly,
so a builtin module reads no `package.json` and wins over an installed package or a self-reference of that name;
the `.js` spelling of a builtin module's specifier (`@fuzdev/gro/process.js`), and the specifier of a host module not built on `fgro` yet
(`@fuzdev/gro/fs.ts`, `spawn`, `net`, and `watch`, each with its `.js` spelling), are refused the same way, naming the one to import
or that the module isn't on `fgro` yet, never looked up as a package, so none of them changes meaning when its builtin module lands.
So is a respelling of any of these or of a builtin module's specifier, one whose subpath a package's `"exports"` would turn into the same path
(percent-encoding as in `@fuzdev/gro/proces%73.ts`, a query or fragment as in `@fuzdev/gro/fs.js?.ts`, a tab or line break):
the resolver reads its subpath as URL parsing and the path's decoding do, whatever patterns the installed package has.
Any other spelling, like `@fuzdev/gro/process`, is a package specifier like any other.
A builtin module resolves to its URL, the specifier in the `builtin:` scheme (`builtin:@fuzdev/gro/process.ts`),
with no file, root, or origin, and it imports nothing.
That scheme is refused like every other but `file:`, so only the table's specifier reaches a builtin module.

Whether an importer may import one is the loader's, checked when the graph links,
for a static import and `import()` alike:
the run must hold the module's grant, when it needs one, and for now only the project's own modules are granted one,
so a module of an installed or linked package is refused, the message naming the grant and the package
(an `import()` in code compiled from a string has no importer of its own and is refused, [run](run.md#what-loads));
`-A` and an acceptance in the policy file are designed to lift that.
A module of an installed or linked package can't import the project's own modules by name either, so it can't borrow a grant through one
(a bare specifier is the only way across, since relative and absolute ones stay in their root):
it's refused naming the package and the module, by a static import, `import()`, and `import.meta.resolve` alike.
The project importing itself, by self-reference or through a link to its own directory, is unaffected, as is a package's self-reference.
This refuses a workspace sibling or a tool package that imports the app by name;
an acceptance in the policy file naming the package, or workspace support, is what will admit one.
`fgro run` grants `process`, and `fgro eval` imports no builtin module, a grant-free one included ([eval](eval.md)).
`import.meta.resolve` returns a builtin module's URL to any importer, granted or not, since it imports nothing.
Attributes follow the language's rule: `type` is the one key supported, and an attribute under any other key (`with {x: 'y'}`)
is refused before anything resolves, for a static import and `import()`, a file and a builtin module alike.
An import of a builtin module with a `type` (`with {type: 'json'}`) is refused before its grant is checked,
where a file's is refused when it loads, every `type` for now ([run](run.md#what-loads)).
A source-phase import of a builtin module (`import source`, `import.source()`) is refused, since a builtin module has no source.

## Departures from Node

Otherwise the resolver follows Node's algorithm.
Where it departs, it departs on purpose, and always toward failing closed:

- **Every module stays inside its root** (origin containment), checked on its real path, after every symlink.
  A relative or absolute specifier resolves only inside the importer's root:
  the project root for project code, the package's real root for package code.
  It never reaches into a `node_modules` segment below that root, by its real path or as written
  (an in-tree checkout linked into `node_modules` has none in its real path),
  and never into another admitted root nested in it,
  such as an in-tree package reached as `./packages/foo/x.ts` once `foo` was imported by name.
  Each is refused naming the path, the root, and the fix (import it by name),
  except an installed package's own relative import out of its root,
  which is the package reaching outside itself, so the fix is a version of the package that doesn't.
  A package's `"exports"` target stays inside the package's real root by the same rules,
  so a target symlinked out of the package is refused,
  and a self-reference's stays inside both its package's directory
  (the one its `package.json` was found in, so a symlinked `package.json` can't move it)
  and the importer's root.
  When import order would decide a module's root
  (a file reached by a relative specifier, then its package admitted by name around it),
  the import fails naming both roots, never choosing one;
  when both are packages, one nested in the other holds the file, and the fix is to import it through only one.
  `import.meta.resolve` runs the same checks and admits nothing,
  so `import(import.meta.resolve('foo'))` for an installed `foo` is a path into `node_modules`, refused with the fix.
  Node allows every crossing, and each one bypasses the package's `"exports"`,
  the boundary a package declares and a grant decision reads its origin from.
  A package's bare import that resolves into the project root is refused too,
  since a bare specifier is the one way left for a package to reach the project and the grants its modules hold
  ([builtin modules](#builtin-modules)).
- **The file loaded is the file resolved.**
  The resolver takes each module file's identity, its device and inode numbers and its birth time,
  when it looks the file up, reading them on the real path without following a symlink there.
  The loader opens the path without blocking, compares the open file's identity with the one resolved,
  and reads the source from that open file, never from the path again,
  so a file, or a directory on its path, swapped between resolving and loading
  (a file renamed over it, a symlink or a FIFO put in its place) is refused:
  the load fails with `cannot load <url>: the file at its path is not the one resolved, so it isn't loaded`.
  The first resolution of a URL is the one checked: resolving it again changes nothing,
  and a module already loaded is served from the module map, not read again.
  Each `package.json` is read the same way, checked against its own lookup.
  In `fgro eval`, which records its graph in full, each `package.json` is read once per run:
  a later resolution that finds the same file there is served the first read's bytes,
  even when it was rewritten in place since,
  and one that finds another file at the path is refused,
  so every resolution in the run parses the same bytes, whose BLAKE3 hash the graph keeps.
  `fgro run` reads it again at each resolution.
  A file removed and made again is told apart by its birth time,
  since ext4 hands the new file the old one's inode number;
  where the filesystem records no birth time, or the new file is made within the same tick of its clock as the original's birth,
  the identity falls back to the device and inode numbers, and the remade file loads.
  One window stays open, inside the lookup itself, between canonicalizing the path and reading the identity:
  a directory on the path swapped there gives the new file's identity, which the loader then finds and loads.
  A last component swapped there for a symlink reads as one and is refused.
  Node reads whatever is at the path when it loads, after checking nothing.
- **The `node_modules` search is bounded by the project root, found first:**
  the first `package.json` at or above the importer, outside `node_modules`,
  that isn't a type marker (a JSON object whose keys are all `"type"` or `"sideEffects"`,
  an empty object included, like `dist/esm/package.json`).
  The walk for it runs before any `node_modules` is looked up,
  and the search then runs from the importer up to the root's own `node_modules`,
  so a `node_modules` planted above a project never resolves.
  Any other `package.json` ends the walk, named or not,
  so an uninstalled project fails instead of resolving a parent directory's packages.
  npm and yarn workspaces, whose packages install at the workspace's root, aren't supported yet.
  Node searches every ancestor's `node_modules`.
- **A file with no project root gets no bare imports except a self-reference,**
  which resolves through its own package's `"exports"` and stays inside its package's directory
  (a module in an installed package importing that package by name).
  With no `package.json` to bound the search, any other bare specifier is refused,
  naming the importer's directory and the fix:
  a `package.json` at the project root, with the package installed in the `node_modules` beside it.
  Its relative imports still resolve.
- **A world-writable directory ends every walk up, and no `package.json` or `node_modules` in it is read:**
  not its `package.json` (as a project root, a `"type"` scope, or a self-reference's scope),
  and not its `node_modules`.
  A package the search finds through one is refused, naming the directory and the fix (remove its other-write bit):
  the `node_modules` it is installed in, its scope's directory (`@scope`), or its own root,
  and, for a linked package (as pnpm's and Deno's stores install them), the real directories holding it
  (each one up to the real `node_modules` it was found through or the importer's own directories,
  or, when its real path passes neither, only as many as its name has segments: the store directory holding it,
  and for a scoped name its `@scope` too, so a checkout linked from under `/tmp` isn't refused for `/tmp` itself),
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
- **No other condition matches:** not `node` or `require` (Node's), nor `browser` or `svelte`; the set grows only by a decision.
- **A pattern's match can't hide a `..` behind a tab or a line break** (`.\t.`), which URL parsing drops,
  where Node substitutes it unchecked and loads what it names outside the package.
- **A pattern's match can't make an empty segment the target doesn't have:**
  a `/` at its edge beside a `/` in the target, as a subpath ending in `/` makes
  (`pkg/x/` through `"./*": "./lib/*/index.js"` is `lib/x//index.js`), or an empty segment inside it,
  is refused naming the pattern and the match,
  where Node substitutes it, with a deprecation warning for the trailing `/` (DEP0155), and loads what it names.
  A target's own empty segment (`./lib//a.js`) still loads, as in Node.
