import { describe, test, expect } from 'vitest';
import { join } from 'node:path';

import { resolve_specifier } from '$lib/resolve_specifier.ts';
import { paths } from '$lib/paths.ts';

const dir = paths.source + 'test/fixtures/';

describe('resolve_specifier', () => {
	test('resolves a specifier to a file that exists with an unknown file extension', async () => {
		const specifier = join(dir, 'test_file.other.ext');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_file.other.ext',
			namespace: undefined,
			raw: false
		});
	});

	test('resolves a TS specifier', async () => {
		const specifier = join(dir, 'test_ts.ts');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});

	test('resolves a TS specifier that does not exist', async () => {
		const specifier = join(dir, 'this_test_ts_does_not_exist.ts');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './this_test_ts_does_not_exist.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});

	test('resolves a `?raw` ts specifier', async () => {
		const path = join(dir, 'test_ts.ts');
		const specifier = path + '?raw';
		const path_id = path;
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id + '?raw',
			specifier,
			mapped_specifier: './test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: true
		});
	});

	test('resolves relative ts specifiers', async () => {
		const path_id1 = join(dir, 'test_ts.ts');
		expect(await resolve_specifier('./test_ts.ts', dir)).toEqual({
			path_id: path_id1,
			path_id_with_querystring: path_id1,
			specifier: './test_ts.ts',
			mapped_specifier: './test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		const path_id2 = join(dir, 'a/b/test_ts.ts');
		expect(await resolve_specifier('./a/b/test_ts.ts', dir)).toEqual({
			path_id: path_id2,
			path_id_with_querystring: path_id2,
			specifier: './a/b/test_ts.ts',
			mapped_specifier: './a/b/test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		const path_id3 = join(dir, '../../test_ts.ts');
		expect(await resolve_specifier('../../test_ts.ts', dir)).toEqual({
			path_id: path_id3,
			path_id_with_querystring: path_id3,
			specifier: '../../test_ts.ts',
			mapped_specifier: '../../test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		const path_id4 = join(dir, '../../test_ts.ts');
		expect(await resolve_specifier('../../test_ts.ts?raw', dir)).toEqual({
			path_id: path_id4,
			path_id_with_querystring: path_id4 + '?raw',
			specifier: '../../test_ts.ts?raw',
			mapped_specifier: '../../test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: true
		});
	});

	test('resolves an extensionless specifier', async () => {
		const specifier = join(dir, 'test_ts');
		const path_id = specifier + '.ts';
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_ts.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});

	test('resolves a js specifier', async () => {
		const specifier = join(dir, 'test_js.js');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_js.js',
			namespace: 'sveltekit_local_imports_js',
			raw: false
		});
	});

	test('resolves a js specifier as ts for a file that does not exist', async () => {
		const specifier = join(dir, 'test_missing.js');
		const path_id = join(dir, 'test_missing.ts');
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_missing.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});

	test('resolves an extensionless specifier for a file that does not exist', async () => {
		const specifier = join(dir, 'test_missing');
		const path_id = specifier + '.ts';
		expect(await resolve_specifier(specifier, dir)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			mapped_specifier: './test_missing.js',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});
});
