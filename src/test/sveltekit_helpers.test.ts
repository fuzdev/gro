import { assert, describe, test, expect } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { has_sveltekit_app, has_sveltekit_library } from '$lib/sveltekit_helpers.ts';
import { SVELTE_PACKAGE_DEP_NAME, SVELTEKIT_DEP_NAME } from '$lib/constants.ts';

const to_package_json = (
	deps: Partial<Pick<PackageJson, 'dependencies' | 'devDependencies' | 'peerDependencies'>>
): PackageJson => ({ name: 'a', version: '0', ...deps });

describe('has_sveltekit_app', () => {
	test('detects SvelteKit as a dep or dev dep', () => {
		expect(
			has_sveltekit_app(to_package_json({ dependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(true);
		expect(
			has_sveltekit_app(to_package_json({ devDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(true);
	});

	test('is not detected with no SvelteKit dependency', () => {
		expect(has_sveltekit_app(to_package_json({})).ok).toBe(false);
	});

	// A package peered on SvelteKit is built to work with one, not to be one -
	// counting the peer would run `vite build` over a library that has no app.
	test('ignores a peer dependency', () => {
		expect(
			has_sveltekit_app(to_package_json({ peerDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(false);
	});
});

describe('has_sveltekit_library', () => {
	test('needs SvelteKit first', async () => {
		const result = await has_sveltekit_library(
			to_package_json({ devDependencies: { [SVELTE_PACKAGE_DEP_NAME]: '2' } })
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTEKIT_DEP_NAME);
	});

	// The packaging dep is checked before the lib directory so a project that isn't a library
	// never reads the Svelte config, which is the only one of the three checks that costs anything.
	test('needs the packaging dep, and reports it before reading the config', async () => {
		const result = await has_sveltekit_library(
			to_package_json({ devDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTE_PACKAGE_DEP_NAME);
	});

	// Resolves this project's own config for the lib directory check, so it covers the whole path.
	test('detects this project', async () => {
		const result = await has_sveltekit_library(
			to_package_json({
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
			})
		);
		expect(result.ok).toBe(true);
	});

	test('ignores peer dependencies', async () => {
		const result = await has_sveltekit_library(
			to_package_json({
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2' },
				peerDependencies: { [SVELTE_PACKAGE_DEP_NAME]: '2' }
			})
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTE_PACKAGE_DEP_NAME);
	});
});
