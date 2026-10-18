# test

Gro integrates [`Vitest`](https://vitest.dev/) for tests:

```bash
gro test # run all tests with Gro's default `.test.` pattern
gro test thing.test somedir test/a.b # run tests matching patterns
```

> Running `gro test [...args]` calls Vitest
> inside Gro's normal [task context](/src/docs/task.md).
> Vitest has native TypeScript support and excellent performance.

> Where it's going: Vitest stays until the [runtime](runtime.md) ships its own test runner near the Node exit,
> running as a subprocess under the run's kernel rules meanwhile.

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
```

[`gro test`](/src/lib/test.task.ts) runs all `*.test.ts`
files in your project by default using the pattern `".test."`.
So to add a new test, create a new file:

```ts
// by convention, create `src/lib/thing.ts`
// to test `src/lib/thing.test.ts`
import {test, expect} from 'vitest';

import {thing} from './thing.ts';

test('the thing', async () => {
	expect(thing).toEqual({expected: true});
});
```

See [the Vitest docs](https://vitest.dev/) for more.
