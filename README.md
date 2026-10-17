# gro <img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" width="32" height="32">

[<img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" align="right" width="192" height="192">](https://gro.fuz.dev/)

> task runner and toolkit extending SvelteKit 🌰 generate, run, optimize

> ⚠️ The TypeScript tool [doesn't support Windows](https://github.com/fuzdev/gro/issues/319), I chose Bash instead;
> the Rust [runtime](#runtime) will.

Gro is a task runner and toolkit
extending [SvelteKit](https://github.com/sveltejs/kit),
[Vite](https://github.com/vitejs/vite),
and [esbuild](https://github.com/evanw/esbuild)
for making web frontends, servers, and libraries with TypeScript.

[`npm i -D @fuzdev/gro`](https://www.npmjs.com/package/@fuzdev/gro)

Gro is a dev tool, not for production use.
It runs on Node today and is becoming its own JS runtime, a Rust binary on V8
(see [Runtime](#runtime)).
It includes:

- [task runner](/src/docs/task.md) that uses the filesystem convention `*.task.ts`
  - lots of [common builtin tasks](/src/docs/tasks.md) that users can easily override and compose
- tools and patterns for
  [developing](/src/docs/dev.md),
  [building](/src/docs/build.md),
  [testing](/src/docs/test.md),
  [deploying](/src/docs/deploy.md),
  and [publishing](/src/docs/publish.md)
  [SvelteKit](https://github.com/sveltejs/kit) apps, library packages, and Node servers
  - integrated [TypeScript](https://github.com/microsoft/typescript),
    [Svelte](https://github.com/sveltejs/svelte),
    and [SvelteKit](https://github.com/sveltejs/kit)
  - defers to SvelteKit and Vite for the frontend and
    [`@sveltejs/package`](https://svelte.dev/docs/kit/packaging) for the library
  - exposes all of its internals in `src/lib`
  - uses [Changesets](https://github.com/changesets/changesets) for versioning and changelogs
  - provides a [Node loader](/src/lib/loader.ts) with a [register hook](/src/lib/register.ts)
    - uses Node's type stripping and supports importing JSON, SvelteKit shims,
      and SSR'd Svelte files in tests/tasks/scripts
    - supports [SvelteKit module imports](https://kit.svelte.dev/docs/modules) for
      `$app` (including `$app/env`) in tasks, tests, Node servers,
      and other code outside of the SvelteKit frontend,
      so you can use SvelteKit patterns everywhere
      (these are best-effort shims, not perfect)
    - supports running TypeScript files directly without a task via `gro run a.ts`
      or `node --import @fuzdev/gro/register.js a.ts`
  - [configurable plugins](/src/docs/plugin.md) to support SvelteKit,
    [auto-restarting Node servers](/src/lib/gro_plugin_server.ts),
    and other external build processes
    - see the [Gro config docs](/src/docs/config.md) and
      [the default config](https://github.com/fuzdev/gro/blob/main/src/lib/gro.config.default.ts)
    - see [`fuz_template`](https://github.com/fuzdev/fuz_template)
      for a simple starter project example, and
      [`fuz_repos`](https://github.com/fuzdev/fuz_repos) for a more complex example with custom tasks
- [testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest)
- codegen by convention with [`gen`](/src/docs/gen.md)
- linting with [ESLint](https://github.com/eslint/eslint)
  (I also maintain [`@ryanatkn/eslint-config`](https://github.com/ryanatkn/eslint-config))
- formatting with [`tsv`](https://github.com/fuzdev/tsv)

## Runtime

> **status**: experimental, design settled, not functional yet -
> `crates/` holds the Rust workspace, and the `fgro` binary parses its arguments and exits

Gro is becoming its own JS runtime: a Rust binary that embeds V8
and runs gro's TS surface (tasks, gen, config) on an engine we own.
It's a clean break from Node, with no compat layer
(see [Where it diverges from Deno](#where-it-diverges-from-deno)).
The migration retires Deno from the Fuz stack first (scripts, confined config eval),
now that Cloudflare has bought Deno and the Deno runtime gets only bug and security fixes
until its end of life in October 2027 (rusty_v8 stays maintained by Cloudflare),
and drops Node much later.
Until then the npm package keeps working on Node:
the runtime proves itself on small use cases before anything switches,
and the binary is named `fgro` until it does.

Its primary verb is `gro run <file>`: one module graph, no ambient authority,
nothing reachable that the graph didn't import or the
[policy file](#what-the-clean-break-opens-up) didn't accept.
`gro eval` is its confined one-shot sibling.
`gro task <path>` is the workflow layer on top: a project task runs by its full path,
never by a searched name, while the builtins stay first-class subcommands (`gro check`, `gro dev`);
the bare `gro <name>` task search goes away, starting with the Node gro.

Why: one runtime we own in place of two we don't,
capability confinement for config eval and build-time code,
one embeddable host for the build tool,
and simplicity: fewer moving parts by construction.
The runtime prioritizes, in order:

1. security: no ambient authority, confinement by construction, a kernel backstop under it
2. web-standards conformance: the WinterTC minimum common API, graded by its WPT subset
3. speed: startup first, since short-lived invocations dominate an agent-driven loop;
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
4. binary size and memory
5. embeddability: a crate first, a binary second

Gro will run the same V8 as Node and Deno, so steady-state compute will be at parity;
the wins are everything around the engine (startup, loading, caching, work avoided).
Feedback is welcome to help navigate these tradeoffs.

### Where it diverges from Deno

If you know Deno, the ingredients are familiar:
V8 through [rusty_v8](https://github.com/denoland/rusty_v8),
web standards, TypeScript without a build step, and permissions.
The runtime is bootstrapped on [deno_core](https://crates.io/crates/deno_core)
and Deno's web-standards extension crates, used thinly behind gro's own seams
so each piece can be rewritten on its own as measurements justify it.
From there it takes the other side of most of Deno's later choices:

- **No Node compat.** No `node:*`, no `npm:`, no CommonJS, no N-API, and no `process` global.
  Anything beyond web standards is an explicit import from a small set of host modules
  ([`fuz_host`](https://github.com/fuzdev/fuz_host): fs, process, spawn, watch).
  Third-party Node tools (Vite, vitest, eslint, svelte-check) run as Node subprocesses
  under the same kernel rules as the invocation that spawned them.
  Bun and Deno each spent years on Node compat; gro declines that work outright.
- **Strict ESM resolution, no legacy algorithm.** Relative and absolute specifiers
  resolve as URLs with mandatory extensions: no probing, no directory index, no `main`.
  Bare specifiers resolve through a package's `exports` with a fixed condition set
  (`gro`, `import`, `default`) and `imports` for `#` subpaths, walking `node_modules`.
  A package without `exports`, or one that's CommonJS, fails loudly with the reason;
  the fix is a version with an ESM build, an ESM bundle of it vendored into the project,
  or running it on Node. Gro never pre-bundles a project's dependencies.
  Files on disk only: no `https:`, `jsr:`, or `npm:` imports.
- **No runtime namespace.** There is no `gro.*` the way there is a `Deno.*`;
  the long-run API target is the
  [WinterTC minimum common API](https://min-common-api.proposal.wintertc.org/),
  and the first builds ship the subset the stack's own code touches.
- **TypeScript by type stripping only.** [tsv](https://github.com/fuzdev/tsv) strips types in-process,
  preserving every position, so stack traces point at the TS source with no source maps.
  Erasable syntax only, like Node's stripper: no enums, namespaces, or parameter properties.
- **No ambient authority.** Permissions are a grant set passed to the runtime at construction,
  empty by default, not process-level flags checked at call time.
  Authority is lexically scoped: a module reaches only what it imports or is handed,
  the way a function reaches only the bindings in scope.
  Host modules are imports and the module loader is the boundary,
  so an ungranted graph fails at link time, not at the first call.
  The global object is the one dynamic channel: the web globals that reach the world
  (`fetch`, `WebSocket`) are declared, never inferred, and absent until then,
  and a net grant names hosts, never "all".
  Pure globals (`URL`, `TextEncoder`, streams, `console`) are always there;
  ops check the scoped cases (reads under a root).
  `-A` is the one explicit escape hatch, never a default.
  Under the grants, confined eval gets a kernel backstop on Linux
  (Landlock for the filesystem and TCP, seccomp for the rest of the socket and
  process surface), installed before V8 starts a thread, so an op-check bug or an
  engine escape still lands in a box.
- **First-party natives linked in, everything else WASM.** tsv and blake3 are crates in the
  binary, exposed as built-in modules with no boundary to cross.
  Third-party native code arrives as WASM, or as an embedder's own host modules.
  FFI (loading a native library from JS) is planned as a grant, denied by default,
  after the first milestones and before the Node exit.
- **A dev-tool runtime, not a toolchain.** Tasks, gen, check, run, and confined eval.
  No `serve`, TCP, or TLS until a consumer needs them; no test runner, package manager,
  formatter, linter, or LSP in the binary (tsv formats, vitest tests, eslint lints).
- **A crate first, a binary second.** `gro_runtime` is a library other Rust binaries embed,
  with a registration API for their own host modules; `gro` is one such binary.
  Grants are a constructor argument, so an embedder is confined without flags.
- **One global binary, pinned per project.** A project declares a compatible range under
  `engines.gro` in `package.json`, and the binary fails hard outside it.
  Svelte, Vite, and SvelteKit stay the project's own dependencies; the binary owns gro,
  never the project's compiler or framework.
  Linux x86_64 first; macOS and Windows builds come before the binary replaces the
  npm package.

### What the clean break opens up

Designed, not built:

- **Least privilege for free.** Host modules are the only path to the OS,
  so a script's required grants are a static property of its import graph.
  A `gro run` of a file that imports only `fuz_util` and the fs module can't spawn or fetch,
  and nobody wrote a line to get that; net is a declaration, so a dependency can't phone
  home from a run that never asked, and a `grants` literal only narrows.
  `typeof fetch` stays the feature check the web already uses,
  and the error for an undeclared one names the grant to add.
  `gro run` and `gro eval` get exactly their file's graph; `gro task` gets the union of
  what the invocation reaches, since a task holds the context (config, `invoke_task`).
  Until inference lands, the CLI passes every host module.
- **A committed policy file.** The source proposes (inferred host modules, plus a static
  `grants` literal naming hosts and the like) and the file authorizes:
  a dependency bump that newly reaches the filesystem, spawn, or the network fails
  `gro check` until a human accepts the diff, the same shape as a lockfile change.
- **Deterministic runs.** Clock and randomness are capabilities (ambient, denied, or fixed),
  so an eval or gen run replays exactly.
  Gen output becomes a memoized function of its module graph and inputs,
  and `gro gen --check` becomes a cache probe.
- **One module graph.** The loader's graph is the only graph:
  it keys caches and gen invalidation, drives watch,
  and lists tasks by parsing instead of executing them.
- **Cheap isolation.** V8 isolates and contexts are cheap in one process,
  so gen files can run in parallel, each confined, with no process spawn;
  trust tiers (project, ecosystem, third party) get their own global object.
- **Startup.** A snapshot of the host, a code cache keyed by content,
  and lazy host init.
- **Fewer platform-isms.** URL resolution with no path separators and argv-only spawn
  (never a shell string) keep platform differences in a handful of files.

## Docs

- developing web frontends, servers, and libraries
  - [config](/src/docs/config.md)
  - [dev](/src/docs/dev.md)
  - [build](/src/docs/build.md) for production
  - [deploy](/src/docs/deploy.md) to a branch, like for GitHub pages
  - [publish](/src/docs/publish.md) to npm
  - API reference at [/docs/api](https://gro.fuz.dev/docs/api)
- [`Task`](/src/docs/task.md) runner
  - builtin [tasks](/src/docs/tasks.md) list
- [testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest)
- [`gen`](/src/docs/gen.md) code generation
- full [docs index](/src/docs#readme)

## Install

> depends on node >=24.14

Typical usage installs [@fuzdev/gro](https://www.npmjs.com/package/@fuzdev/gro)
as a dev dependency:

```bash
npm i -D @fuzdev/gro
npx @fuzdev/gro # note the package is namespaced, don't install `gro`
```

It's handy to install globally too:

```bash
npm i -g @fuzdev/gro
gro
```

## Usage

Gro has a task runner that discovers TypeScript modules with the `.task.` subextension,
and runs them in-process with explicit composition.
Running `gro` with no args prints the tasks
it finds in the current directory along with its builtin tasks:

```bash
gro # prints available tasks - defers to any local gro installation
```

```
Run a task: gro [name]
View help:  gro [name] --help

19 tasks in gro:

build      build the project
changeset  call changeset with gro patterns
check      check that everything is ready to commit
clean      remove temporary dev and build files, and optionally prune git branches
commit     commit and push to a new branch
deploy     deploy to a branch
dev        start SvelteKit and other dev plugins
format     format source files
gen        run code generation scripts
lint       run eslint
publish    bump version, publish to the configured registry, and git push
reinstall  refreshes package-lock.json with the latest and cleanest deps
release    publish and deploy
resolve    diagnostic that logs resolved filesystem info for the given input paths
run        execute a file with the loader, like `node` but works for TypeScript
sync       run `gro gen`, update `package.json`, and optionally install packages to sync up
test       run tests with vitest
typecheck  run svelte-check or tsc on the project without emitting any files
upgrade    upgrade deps
```

To run tasks, Gro matches your CLI input against its filesystem conventions.
It tries to do the right thing, where right is helpful but not surprising,
with some magic but not too much:

```bash
gro # displays all available tasks matching `src/lib/**/*.task.ts` and Gro's builtins
gro a # tries to run `src/lib/a.task.ts`, then `./a.task.ts`, then Gro's builtin if one exists
gro a --help # displays docs for the "a" task and its args, works for every task
gro some/dir # lists all tasks inside `src/lib/some/dir`
gro some/file # runs `src/lib/some/file.task.ts`
gro some/file.task.ts # same as above
```

Gro can also run non-task TypeScript files directly
with [the `gro run` task](/src/lib/run.task.ts) or [register hook](/src/lib/register.ts):

```bash
gro run foo.ts
node --import @fuzdev/gro/register.js foo.ts
```

Or programmatically:

```js
// myfile.js
import {register} from 'node:module';
register('@fuzdev/gro/loader.ts', import.meta.url);
await import('./foo.ts');
```

Gro has a number of builtin tasks that you can run with the CLI.
To learn more [see the task docs](/src/docs/task.md)
and [the generated task index](/src/docs/tasks.md).

```bash
gro dev # start developing in watch mode
gro dev -- vite --port 3003 # forward args by separating sections with --
```

```bash
gro build # build everything for production
```

[Testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest),
including shims for [SvelteKit modules](https://kit.svelte.dev/docs/modules):

```bash
gro test # run all tests for `*.test.ts` files with `vitest`
gro test filepattern1 some.test another.test
gro test optional_pattern -t "optional search string for test name"
gro test -- vitest --forwarded_args 'to vitest'
```

Check all the things:

```bash
gro check # does all of the following:
gro typecheck # svelte-check with tsc fallback
gro test # run tests
gro gen --check # ensure generated files are current
gro format --check # ensure everything is formatted
gro lint # eslint
```

For a usage example see [the `check.yml` CI config](.github/workflows/check.yml).

Formatting with [`tsv`](https://github.com/fuzdev/tsv):

```bash
gro format # format all of the source files using tsv
gro format --check # check that all source files are formatted
```

Codegen with [`gen`](/src/docs/gen.md):

```bash
gro gen # run codegen for all `*.gen.*` files
gro gen --check # error if any generated files are new or different
```

To deploy: (also see [`src/docs/deploy.md`](/src/docs/deploy.md))

```bash
gro deploy # build and push to the `deploy` branch
```

To publish: (also see [`src/docs/publish.md`](/src/docs/publish.md))

```bash
gro publish # flush changeset to changelog, bump version, publish to npm, and git push
```

More:

```bash
gro clean # delete all build artifacts from the filesystem
gro clean --sveltekit --nodemodules --git # also deletes dirs and prunes git branches
gro upgrade excluded-dep-1 excluded-dep-2 # npm updates to the latest everything
gro --version # print the Gro version
```

For more see [the tasks index](/src/docs/tasks.md),
[the task feature docs](/src/docs/task.md), and [the docs index](/src/docs/README.md).

## Develop

```bash
npm i
npm run bootstrap # build and link `gro` without itself - needed only once
gro build # same as `npm run bootstrap` when the `gro` CLI is available
gro test # make sure everything looks good - same as `npm test`
gro test some.test another.test

# use your development version of `gro` locally in another project:
gro build # updates the `gro` CLI, same as `npm run bootstrap`
cd ../otherproject
npm link ../gro # from `otherproject/`
gro build # from `../gro` on changes
```

## Credits 🐢<sub>🐢</sub><sub><sub>🐢</sub></sub>

Gro builds on
[TypeScript](https://github.com/microsoft/TypeScript) ∙
[Svelte](https://github.com/sveltejs/svelte) ∙
[SvelteKit](https://github.com/sveltejs/kit) ∙
[Vite](https://github.com/vitejs/vite) ∙
[esbuild](https://github.com/evanw/esbuild) ∙
[Vitest](https://github.com/vitest-dev/vitest) ∙
[chokidar](https://github.com/paulmillr/chokidar) ∙
[@fuzdev/fuz_util](https://github.com/fuzdev/fuz_util) ∙
[@fuzdev/fuz_schema](https://github.com/fuzdev/fuz_schema) ∙
[ESLint](https://github.com/eslint/eslint) ∙
[tsv](https://github.com/fuzdev/tsv) ∙
[svelte-check](https://github.com/sveltejs/language-tools/tree/master/packages/svelte-check) &
[more](package.json)

## Contributing

[fuz.dev/contributing](https://www.fuz.dev/contributing)

## License [🐦](https://wikipedia.org/wiki/Free_and_open-source_software)

[0BSD](LICENSE)
