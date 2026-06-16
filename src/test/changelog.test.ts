import {test, expect} from 'vitest';
import {Logger} from '@fuzdev/fuz_util/log.ts';
import {readFile, writeFile} from 'node:fs/promises';
import type {FetchValueCache} from '@fuzdev/fuz_util/fetch.ts';

import {update_changelog} from '$lib/changelog.ts';
import {load_from_env} from '$lib/env.ts';

const log = new Logger();

const token = load_from_env('SECRET_GITHUB_API_TOKEN');
if (!token) {
	log.warn('the env var SECRET_GITHUB_API_TOKEN was not found, so API calls with be unauthorized');
}

const fixture_path = 'src/test/fixtures/changelog_example.md';

// TODO ideally this is just a ts file, but there's a problem where building outputs a `.d.ts` file
// when importing from src/test/fixtures (fix in SvelteKit/Vite/tsconfig?) and I want to keep it in src/test/fixtures
const changelog_cache_fixture: FetchValueCache = new Map(
	JSON.parse(await readFile('src/test/fixtures/changelog_cache.json', 'utf8')),
);

test('update_changelog', async () => {
	const original = await readFile(fixture_path, 'utf8');
	const result = await update_changelog(
		'fuzdev',
		'gro',
		fixture_path,
		token,
		log,
		changelog_cache_fixture,
	);
	const updated = await readFile(fixture_path, 'utf8');
	await writeFile(fixture_path, original, 'utf8');
	expect(result).toBeTruthy();
	expect(updated).toBe(
		`# @fuzdev/gro

## 0.6.0

### Minor Changes

- duplicate 1 ([#429](https://github.com/fuzdev/gro/pull/429))
- duplicate 2 ([#429](https://github.com/fuzdev/gro/pull/429))
- abc ([#437](https://github.com/fuzdev/gro/pull/437))
  - 123
  - 123
  - 123

### Patch Changes

- abc ([5e94cd4](https://github.com/fuzdev/gro/commit/5e94cd4))

## 0.5.2

### Patch Changes

- abc ([e345eaa](https://github.com/fuzdev/gro/commit/e345eaa))

## 0.5.1

### Patch Changes

- abc ([094279d](https://github.com/fuzdev/gro/commit/094279d))

## 0.5.0

### Minor Changes

- abc ([f6133f7](https://github.com/fuzdev/gro/commit/f6133f7))

## 0.4.3

### Patch Changes

- abc ([54b65ec](https://github.com/fuzdev/gro/commit/54b65ec))

## 0.4.2

### Patch Changes

- abc ([80365d0](https://github.com/fuzdev/gro/commit/80365d0))

## 0.4.1

### Patch Changes

- abc ([3d84dfd](https://github.com/fuzdev/gro/commit/3d84dfd))
- abc ([fc64b77](https://github.com/fuzdev/gro/commit/fc64b77))

## 0.4.0

### Minor Changes

- abc ([#434](https://github.com/fuzdev/gro/pull/434))
  - 123
  - 123

### Patch Changes

- abc ([#434](https://github.com/fuzdev/gro/pull/434))

## 0.3.1

- e

## 0.3.0

- b2
- c2
- d2

## 0.2.0

- a2

## 0.1.2

- e

## 0.1.1

- b
- c
- d

## 0.1.0

- a
`,
	);
});
