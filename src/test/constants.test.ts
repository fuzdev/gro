import { describe, test, assert } from 'vitest';

import { SVELTE_RUNES_MATCHER } from '#lib/constants.ts';

describe('SVELTE_RUNES_MATCHER', () => {
	test('matches `.svelte.` in the file name, then any segments, then `.js` or `.ts`', () => {
		for (const path of [
			'/a/state.svelte.ts',
			'/a/state.svelte.js',
			'/a/state.svelte.test.ts',
			'/a/state.svelte.a.b.js',
			'file:///a/b.c/state.svelte.ts'
		]) {
			assert.ok(SVELTE_RUNES_MATCHER.test(path), path);
		}
	});

	test('does not match components, other extensions, or `.svelte.` in a directory', () => {
		for (const path of [
			'/a/Component.svelte',
			'/a/state.ts',
			'/a/state.svelte.mts',
			'/a/state.svelte.json',
			'/a/svelte.ts',
			'/a/x.svelte.dir/state.ts',
			'/a/x.svelte.\\state.ts'
		]) {
			assert.ok(!SVELTE_RUNES_MATCHER.test(path), path);
		}
	});
});
