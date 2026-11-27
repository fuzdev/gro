# test

Gro integrates [`Vitest`](https://vitest.dev/) for tests:

```bash
gro test # run all tests with Gro's default `.test.` pattern
gro test thing.test somedir test/a.b # run tests matching patterns
gro test -- --watch --bail 1 # forward args to Vitest verbatim
gro check -- vitest --bail 1 # forward to Vitest from a task that invokes `test`
```

The first section after a `--` goes to Vitest, unless it starts with `gro` or `vitest`,
only when the CLI invoked `gro test` itself or a task invoking it passes it as `implicit_rest_args`, and only up to the next `--`;
a `-- vitest` section reaches Vitest wherever `test` is invoked.
See [task args forwarding](/src/docs/task.md#task-args-forwarding).

> Running `gro test [...args]` calls Vitest
> inside Gro's normal [task context](/src/docs/task.md).
> Vitest has native TypeScript support and excellent performance.

> Where it's going: Vitest stays until the [runtime](runtime.md) ships its own test runner near the Node exit,
> running meanwhile as a subprocess, on the binary in a kernel box of its own ([box](box.md#a-box-for-each-tool)).

Like other tasks, use `--help` to see the args info:

```bash
gro test --help
```

outputs:

```
gro test: run tests with vitest

[...args]          Array<string>      ['.test.']              file patterns to filter tests
dir                string             '/path/to/project/src/' working directory for tests
fail_without_tests boolean            false                   opt out of `passWithNoTests`
t                  string | undefined undefined               search pattern for test names, same as vitest -t and --testNamePattern
no-sync            boolean            false                   opt out of syncing SvelteKit before the tests
```

[`gro test`](/src/lib/test.task.ts) runs all `*.test.ts`
files in your project by default using the pattern `".test."`.
So to add a new test, create a new file:

```ts
// by convention, create `src/test/thing.test.ts`
// to test `src/lib/thing.ts`
import {test, expect} from 'vitest';

import {thing} from '#lib/thing.ts';

test('the thing', async () => {
	expect(thing).toEqual({expected: true});
});
```

See [the Vitest docs](https://vitest.dev/) for more.

Gro supports Vitest 5, the range `^5` of its `vitest` peer dependency.

## SvelteKit's files, synced first

In a SvelteKit project `gro test` runs `svelte-kit sync` before Vitest, as `gro typecheck` does,
and `gro check` turns that off with `--no-sync` once its typecheck has synced.
SvelteKit's plugin in Vitest then finds the tsconfig and types it generates in `node_modules/$app` current and writes nothing there,
so on the binary the [box](box.md#what-a-commands-box-admits) lets no tool but `svelte-kit` write `$app`:
a later typecheck, `dev`, `build`, and the editor trust what's there,
and a file planted beside the generated ones, like a `package.json` whose `exports` redirect `$app/tsconfig`, survives every sync.
`gro test --no-sync` skips the sync, so on the binary it fails on an unsynced tree, where Vitest would have to write `$app`.

## Vitest's module cache

`gro test` passes Vitest `--fsModuleCache=false`, turning off its `fsModuleCache`,
on in later Vitest 5 releases by default, which keeps transformed modules in `node_modules/.vitest-cache` between runs.
Vitest runs a cached module's code as read in every later run,
until the module's source, the config, or the lockfile changes,
so code a test wrote into the cache would run in the next Vitest run, an unconfined one included.
On the Fuz stack's suites, a warm cache measured no faster than a cold one, within the runs' noise.
The flag is not a protection: it keeps `gro test` from reading or making the cache,
and on the Node Gro test code can still write one, which a plain `vitest` run, like an editor's, would read.
On the binary the run's [box](box.md#what-a-commands-box-admits) keeps `node_modules` read-only but for its tool directories, so a test there can't.

A forwarded `fsModuleCache` flag decides instead, in camel or kebab case,
like `gro test -- --fsModuleCache` or `--no-fs-module-cache`.
In Vitest 5, `--fs-module-cache=false` turns the cache on, and `--no-fs-module-cache` turns it off.
The deprecated `--experimental.fsModuleCache` isn't one, and Gro's flag overrides it.

## When Vitest isn't declared

Gro detects Vitest from the project's `dependencies` and `devDependencies`.
A `peerDependencies` entry doesn't count, as a package peered on Vitest is built to work with one,
the same rule as Gro's SvelteKit detection, and `gro lint` follows it for ESLint
([task](./task.md#wrap-one-of-gros-builtin-tasks)).

`gro test` fails when Vitest isn't declared, and when it's declared but not installed
in the project's `node_modules/.bin`, as a copy on `PATH` never runs in its place.

`gro check` runs its test step only when Vitest is declared or args are forwarded to the tests,
a `-- gro test` section or a nonempty `-- vitest` one.
Without either, the step is skipped with a log line, unless the project has test files,
and then `gro check` fails naming them, so a repo that loses its Vitest dependency can't pass without its tests.
A test file is one `gro test` would run with its defaults and Vitest's:
under `src/`, a file name matching Vitest's default `include`, a path below `src/` containing Gro's `.test.` pattern,
and no `node_modules` or `.git` directory on the way, even in a `build/` or `.`-prefixed directory.
Symlinks are followed as Vitest follows them, out of the project too, and a dangling one is skipped.
A search that fails, like a directory that can't be read or a symlink that loops on itself, fails `gro check`.
A custom Vitest config that narrows what runs can make `gro check` report files `gro test` wouldn't run;
`gro check --no-test` skips the step without looking.
