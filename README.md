# gro <img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" width="32" height="32">

[<img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" align="right" width="192" height="192">](https://gro.fuz.dev/)

> hermetic JS runtime and toolchain 🌰 generate, run, optimize

> ⚠️ The TypeScript tool [doesn't support Windows](https://github.com/fuzdev/gro/issues/319), I chose Bash instead;
> the Rust [runtime](#runtime) will.

Gro is becoming a hermetic JS runtime, a Rust binary on V8:
what a run can reach and what it depends on are static facts about its import graph,
so a change in authority is a reviewed diff, like a lockfile,
and a run replays and caches by construction.
It's built for code you didn't write: your dependencies and your agents.
The runtime is experimental (see [Runtime](#runtime)).

Today Gro is a task runner and toolkit
extending [SvelteKit](https://github.com/sveltejs/kit),
[Vite](https://github.com/vitejs/vite),
and [esbuild](https://github.com/evanw/esbuild)
for making web frontends, servers, and libraries with TypeScript.
It runs on Node and is a dev tool, not for production use.

[`npm i -D @fuzdev/gro`](https://www.npmjs.com/package/@fuzdev/gro)

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

> **status**: experimental, direction settled, design in progress -
> `crates/` holds the Rust workspace, and the `fgro` binary parses its arguments and exits

Gro is becoming its own JS runtime: a Rust binary that embeds V8,
built on web standards instead of Node compat, and hermetic by design.
A module reaches only what it imports or is handed,
the way a function reaches only the bindings in scope,
so what a run can reach is a static fact about its import graph,
reviewed like a lockfile.
Clock and randomness are capabilities too,
so what a run depends on is known, and a run replays and caches by construction.

It starts by running gro's own TS surface (tasks, gen, config)
and grows into the toolchain for the whole Fuz stack,
including the server side of [fuz_app](https://github.com/fuzdev/fuz_app),
which keeps supporting Node and Bun beside it.
Until the binary replaces the npm package, the TypeScript version keeps working on Node
and the binary is named `fgro` (see the [roadmap](#roadmap)).

Its primary verb is `gro run <file>`: one module graph, no ambient authority,
nothing reachable that the graph didn't import or the
[policy file](#secure-by-default) didn't accept.
`gro eval` is its confined one-shot sibling.
`gro task <path>` is the workflow layer on top: a project task runs by its full path,
never by a searched name, while the builtins stay first-class subcommands (`gro check`, `gro dev`).

Why: confinement and replay for code you didn't write,
one runtime we own in place of two we don't,
and one embeddable host for the toolchain.
The runtime prioritizes, in order:

1. security: no ambient authority, confinement by construction, a kernel backstop under it
2. web-standards conformance: the WinterTC minimum common API, graded by its WPT subset
3. speed: startup first, since short-lived invocations dominate an agent-driven loop;
   measured against Node, Deno, and Bun on the same workloads,
   with no regression against its own history accepted
4. binary size and memory
5. embeddability: published crates with a registration API, the binary being one embedder

Staying simple and small is overarching, and the security story depends on it:
a surface small enough to audit, where every path to the OS is an import or a declared global.
Gro will run the same V8 as Node and Deno, so steady-state compute will be at parity;
the wins are everything around the engine (startup, loading, caching, work avoided).
Feedback is welcome to help navigate these tradeoffs.

The design in detail, how it compares to Deno, and what the hermetic design opens up
are in [the runtime docs](/src/docs/runtime.md).

### Secure by default

Code starts with nothing and gets only what it's granted.
There's no flag to remember and no mode to opt into:
the default is the confined case, and widening it is a visible, human act.
Designed, not yet built, a run gets:

- **without asking:** what project code reaches through the host modules it imports,
  scoped to reads under the project root, writes to declared build roots,
  and the env names it declares
- **only after a human accepts it:** the network (named hosts, never "all"),
  spawning processes, reads outside the project,
  and any authority reached through a dependency
- **never short of `-A`:** `.env*` files except through the env module,
  credential files (`~/.ssh`, `.git/config`), and writes under `.git` or `node_modules`
- **for `gro eval`:** its own module graph, read-only, with no network, writes, env, or spawn,
  and the JIT off
- **for native code:** never the runtime's process, not even under `-A`;
  FFI is a grant that loads the library in its own confined process

A script's needs are a static property of its import graph,
so least privilege costs nothing to write:
a file that imports only `fuz_util` and the fs module can't spawn or fetch.
The source proposes (inferred host modules, plus a static `grants` literal naming hosts
and the like) and a committed policy file authorizes:
a dependency bump that newly reaches the filesystem, spawn, or the network fails
`gro check` until a human accepts the diff, the same shape as a lockfile change.

The exceptions are explicit and labeled.
`-A` is the one escape hatch, never a default.
`publish` and `deploy` push and use credentials, so they're named unconfined phases,
shown that way in the policy diff, with credentials passed to the one process that needs them.
The kernel backstop is Linux-only until the other platforms get an equivalent.

### Threat model

Grants contain overreach:
a dependency, a task, or an agent that reaches for authority nobody accepted
fails before it runs (an unaccepted host module fails at link time,
an undeclared global isn't there), and the widening shows up as a diff in review.
V8 itself isn't treated as the boundary against code built to escape it.
An engine exploit lands in the kernel backstop,
which applies the same grants to the whole invocation and every child process,
and `gro eval` and FFI each run in a process of their own.
Code that must be treated as hostile belongs in WASM or a separate process,
not under a looser grant.
Outside the model: the unconfined phases (`publish`, `deploy`, `-A`),
and on macOS and Windows, anything past the runtime's own checks
until those platforms get a kernel equivalent.

### Roadmap

The first target is retiring Deno from the Fuz stack:
Cloudflare has bought Deno, and the Deno runtime gets only bug and security fixes
until its end of life in October 2027 (rusty_v8 stays maintained by Cloudflare).
Node leaves much later. Until then, third-party Node tools (Vite, vitest, eslint, svelte-check)
run as Node subprocesses under the invocation's kernel rules.

- [x] Rust workspace and benchmark harness, with baselines against Node, Deno, and Bun
- [ ] `gro run`: ESM from disk on V8, web-standard globals, a startup snapshot
- [ ] TypeScript by tsv's type strip, strict package resolution
- [ ] `gro eval`: confined, deterministic one-shot eval of a TS module to JSON
      under the kernel backstop, for any tool that evaluates config;
      it replaces Deno for config eval in the Fuz stack
- [ ] the Deno exit: config eval on `gro eval`, the stack's other Deno uses on Node
- [ ] startup: a tuned snapshot, a content-keyed code cache, lazy host init
- [ ] on the Node gro: `gro task <path>` replaces the bare `gro <name>` task search,
      the loader turns strict, and `gro check` infers each task's grants
      and diffs them against the committed policy file, reporting before anything enforces it
- [ ] gro on gro: tasks, gen, and check run on the runtime, enforcing the policy file
      (until then the CLI passes every host module; the network stays declared)
- [ ] the Svelte compiler in-engine
- [ ] macOS and Windows builds
- [ ] binary-only: `fgro` becomes `gro`, the npm package carries types, and the crates publish
- [ ] bundled dev and build on the same host, and Vite leaves the loop
- [ ] serving, a test runner, and types from the binary:
      fuz_app's server side runs on gro, and Node leaves the stack's toolchain

Future (unknown order):

- FFI as a grant, in its own confined process
- deterministic tasks and gen: clock and randomness fixed or seeded,
  gen as a memoized function of its graph (`gro eval` denies both from the start)
- owning the core: deno_core's pieces replaced one at a time, measured
- `fetch`, `crypto.subtle`, and `WebSocket` as consumers need them
- size: filtered ICU data, V8 build flags

Maybe later or never:

- per-trust-tier V8 contexts
- a custom V8 build

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
- the experimental [runtime](/src/docs/runtime.md)
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
with some magic but not too much
(the name search is going away: project tasks will run as `gro task <path>`,
see the [roadmap](#roadmap)):

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
