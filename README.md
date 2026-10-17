# gro <img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" width="32" height="32">

[<img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" align="right" width="192" height="192">](https://gro.fuz.dev/)

> hermetic JS runtime and toolchain 🌰 generate, run, optimize

> ⚠️ The TypeScript tool [doesn't support Windows](https://github.com/fuzdev/gro/issues/319), I chose Bash instead;
> the Rust [runtime](/src/docs/runtime.md) will.

Today, Gro is a task runner and toolkit
extending [SvelteKit](https://github.com/sveltejs/kit),
[Vite](https://github.com/vitejs/vite),
and [esbuild](https://github.com/evanw/esbuild)
for making web frontends, servers, and libraries with TypeScript.
It runs on Node and is a dev tool, not for production use.

[`npm i -D @fuzdev/gro`](https://www.npmjs.com/package/@fuzdev/gro)

Gro is becoming a hermetic JS runtime: a Rust binary on V8,
built on web standards instead of Node compat.
What a run can reach and what it depends on are static facts about its import graph,
so a change in authority is a reviewed diff, like a lockfile,
and a run replays and caches by construction.
It's built for code you didn't write: your dependencies and your agents,
in JS or in anything compiled to WASM, under the same grants.
The runtime is experimental;
its roadmap, rationale, security model, and design are in [the runtime docs](/src/docs/runtime.md).

Why a task runner grows a runtime:
the Fuz stack ran on Node and Deno,
and used Deno's permission flags to sandbox the config it evaluates.
When Cloudflare bought Deno and put the runtime on bug and security fixes
until its end of life in October 2027, everything on it had to move.
The scripts could go to Node; the sandbox was worth owning.
Gro already runs tasks, gen files, and config in-process,
so the runtime is that loader becoming the host,
on [rusty_v8](https://github.com/denoland/rusty_v8) (which Cloudflare keeps maintaining)
and [deno_core](https://crates.io/crates/deno_core),
smaller than either runtime it replaces:
no ambient authority, where every path to the OS is an import or a declared global.
Code that needs Node keeps running on Node.

It includes:

- a [task runner](/src/docs/task.md) that discovers `*.task.ts` modules by filesystem convention,
  with many [builtin tasks](/src/docs/tasks.md) that projects override and compose
- [dev](/src/docs/dev.md), [build](/src/docs/build.md), [test](/src/docs/test.md),
  [deploy](/src/docs/deploy.md), and [publish](/src/docs/publish.md) workflows
  for SvelteKit apps, library packages, and Node servers,
  deferring to SvelteKit and Vite for the frontend and
  [`@sveltejs/package`](https://svelte.dev/docs/kit/packaging) for the library
- a [Node loader](/src/lib/loader.ts) with a [register hook](/src/lib/register.ts)
  that runs TypeScript, JSON, and SSR'd Svelte files directly (`gro run a.ts`),
  with best-effort shims for [SvelteKit's `$app` modules](https://kit.svelte.dev/docs/modules)
  in tasks, tests, and Node servers
- [plugins](/src/docs/plugin.md) for SvelteKit,
  [auto-restarting Node servers](/src/lib/gro_plugin_server.ts),
  and other build processes, configured in an optional [`gro.config.ts`](/src/docs/config.md)
  (see [the default config](https://github.com/fuzdev/gro/blob/main/src/lib/gro.config.default.ts),
  [`fuz_template`](https://github.com/fuzdev/fuz_template) for a simple starter, and
  [`fuz_repos`](https://github.com/fuzdev/fuz_repos) for custom tasks)
- codegen by convention with [`gen`](/src/docs/gen.md)
- [testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest),
  linting with [ESLint](https://github.com/eslint/eslint)
  (I also maintain [`@ryanatkn/eslint-config`](https://github.com/ryanatkn/eslint-config)),
  and formatting with [`tsv`](https://github.com/fuzdev/tsv)
- all of its internals exported from `src/lib`,
  and [Changesets](https://github.com/changesets/changesets) for versioning and changelogs

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
with some magic but not too much.
The name search is going away: project tasks will run as `gro task <path>`
(see the [roadmap](/src/docs/runtime.md#roadmap)).

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
