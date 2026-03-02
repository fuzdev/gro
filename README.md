# gro <img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" width="32" height="32">

[<img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" align="right" width="192" height="192">](https://gro.fuz.dev/)

> task runner and toolkit extending SvelteKit 🌰 generate, run, optimize

[`npm i -D @fuzdev/gro`](https://www.npmjs.com/package/@fuzdev/gro)

> ⚠️ I still use Gro heavily but I'm transitioning to Rust-based tooling
> with [Fuz](https://github.com/fuzdev) using [Deno](https://github.com/denoland) as a sidecar.
> I consider Gro deprecated but there should be a migration path.
> Please open issues if you need help.

> ⚠️[Windows won't be supported](https://github.com/fuzdev/gro/issues/319), I chose Bash instead.

Docs at [gro.fuz.dev/docs](https://gro.fuz.dev/docs) and [src/docs](./src/docs).

Need help or want to share thoughts? See the
[issues](https://github.com/fuzdev/gro/issues) and
[discussions](https://github.com/fuzdev/gro/discussions).

## about

Gro is a task runner and toolkit
extending [SvelteKit](https://github.com/sveltejs/kit),
[Vite](https://github.com/vitejs/vite),
and [esbuild](https://github.com/evanw/esbuild)
for making web frontends, servers, and libraries with TypeScript.
It's a dev tool, not for production use.
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
  - exposes all of its internals in `$lib`
  - uses [Changesets](https://github.com/changesets/changesets) for versioning and changelogs
  - provides a [Node loader](/src/lib/loader.ts) with a [register hook](/src/lib/register.ts)
    - uses Node's type stripping and supports importing JSON, SvelteKit shims,
      and SSR'd Svelte files in tests/tasks/scripts
    - supports [SvelteKit module imports](https://kit.svelte.dev/docs/modules) for
      `$lib`, `$env`, and `$app` in tasks, tests, Node servers,
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
    - see [`fuz_template`](https://github.com/fuz-dev/fuz_template)
      for a simple starter project example, and
      [`@feltjs/felt`](https://github.com/feltjs/felt) for a more complex example with custom tasks
- [testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest)
- codegen by convention with [`gen`](/src/docs/gen.md)
- linting with [ESLint](https://github.com/eslint/eslint)
  (I also maintain [`@feltjs/eslint-config`](https://github.com/feltjs/eslint-config))
- formatting with [Prettier](https://github.com/prettier/prettier)

## docs

- early API docs at [/docs/api](https://gro.fuz.dev/docs/api)
- developing web frontends, servers, and libraries
  - [config](/src/docs/config.md)
  - [dev](/src/docs/dev.md)
  - [build](/src/docs/build.md) for production
  - [deploy](/src/docs/deploy.md) to a branch, like for GitHub pages
  - [publish](/src/docs/publish.md) to npm
- [`Task`](/src/docs/task.md) runner
  - builtin [tasks](/src/docs/tasks.md) list
- [testing](/src/docs/test.md) with [`vitest`](https://github.com/vitest-dev/vitest)
- [`gen`](/src/docs/gen.md) code generation
- full [docs index](/src/docs#readme)

## install

> depends on node >=20.12

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

## usage

Gro has a task runner that discovers and runs TypeScript modules with the `.task.` subextension.
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
register('@fuzdev/gro/loader.js', import.meta.url);
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

Formatting with [`prettier`](https://github.com/prettier/prettier):

```bash
gro format # format all of the source files using Prettier
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

## develop

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

## credits 🐢<sub>🐢</sub><sub><sub>🐢</sub></sub>

Gro builds on
[TypeScript](https://github.com/microsoft/TypeScript) ∙
[Svelte](https://github.com/sveltejs/svelte) ∙
[SvelteKit](https://github.com/sveltejs/kit) ∙
[Vite](https://github.com/vitejs/vite) ∙
[esbuild](https://github.com/evanw/esbuild) ∙
[Vitest](https://github.com/vitest-dev/vitest) ∙
[chokidar](https://github.com/paulmillr/chokidar) ∙
[zod](https://github.com/colinhacks/zod) ∙
[@fuzdev/fuz_util](https://github.com/fuzdev/fuz_util) ∙
[ESLint](https://github.com/eslint/eslint) ∙
[Prettier](https://github.com/prettier/prettier) ∙
[svelte-check](https://github.com/sveltejs/language-tools/tree/master/packages/svelte-check) &
[more](package.json)

## license [🐦](https://wikipedia.org/wiki/Free_and_open-source_software)

[MIT](LICENSE)
