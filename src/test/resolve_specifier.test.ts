import { describe, test, expect, assert } from 'vitest';
import { join } from 'node:path';

import { resolve_specifier, to_specifier_path_id } from '#lib/resolve_specifier.ts';
import { paths } from '#lib/paths.ts';

const dir = paths.source + 'test/fixtures/';
const importer = join(dir, 'importer.ts');

const assert_refused = async (specifier: string, expected: string): Promise<void> => {
	let error;
	try {
		await resolve_specifier(specifier, importer);
	} catch (e) {
		error = e;
	}
	assert(error instanceof Error, `expected '${specifier}' to be refused`);
	expect(error.message).toBe(`cannot resolve '${specifier}' from ${importer}: ${expected}`);
};

describe('resolve_specifier', () => {
	test('resolves a specifier to a file that exists with an unknown file extension', async () => {
		const specifier = join(dir, 'test_file.other.ext');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, importer)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			namespace: undefined,
			raw: false
		});
	});

	test('resolves an extensionless specifier naming a file that exists', async () => {
		const specifier = './modules/some_test_json_without_extension';
		const path_id = join(dir, specifier);
		expect(await resolve_specifier(specifier, importer)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			namespace: undefined,
			raw: false
		});
	});

	test('resolves a TS specifier', async () => {
		const specifier = join(dir, 'test_ts.ts');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, importer)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
	});

	test('resolves a `?raw` ts specifier', async () => {
		const path = join(dir, 'test_ts.ts');
		const specifier = path + '?raw';
		const path_id = path;
		expect(await resolve_specifier(specifier, importer)).toEqual({
			path_id,
			path_id_with_querystring: path_id + '?raw',
			specifier,
			namespace: 'sveltekit_local_imports_ts',
			raw: true
		});
	});

	test('resolves relative ts specifiers', async () => {
		const path_id1 = join(dir, 'test_ts.ts');
		expect(await resolve_specifier('./test_ts.ts', importer)).toEqual({
			path_id: path_id1,
			path_id_with_querystring: path_id1,
			specifier: './test_ts.ts',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		const path_id2 = join(dir, 'modules/some_test_ts.ts');
		expect(await resolve_specifier('./modules/some_test_ts.ts', importer)).toEqual({
			path_id: path_id2,
			path_id_with_querystring: path_id2,
			specifier: './modules/some_test_ts.ts',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		const path_id3 = join(dir, 'test_ts.ts');
		expect(await resolve_specifier('../fixtures/test_ts.ts', importer)).toEqual({
			path_id: path_id3,
			path_id_with_querystring: path_id3,
			specifier: '../fixtures/test_ts.ts',
			namespace: 'sveltekit_local_imports_ts',
			raw: false
		});
		expect(await resolve_specifier('../fixtures/test_ts.ts?raw', importer)).toEqual({
			path_id: path_id3,
			path_id_with_querystring: path_id3 + '?raw',
			specifier: '../fixtures/test_ts.ts?raw',
			namespace: 'sveltekit_local_imports_ts',
			raw: true
		});
	});

	test('resolves a js specifier', async () => {
		const specifier = join(dir, 'test_js.js');
		const path_id = specifier;
		expect(await resolve_specifier(specifier, importer)).toEqual({
			path_id,
			path_id_with_querystring: path_id,
			specifier,
			namespace: 'sveltekit_local_imports_js',
			raw: false
		});
	});

	test('refuses a TS specifier that does not exist', async () => {
		await assert_refused('./this_test_ts_does_not_exist.ts', 'no such file');
	});

	test('a refusal carries the code Node uses for a missing module', async () => {
		let error;
		try {
			await resolve_specifier('./this_test_ts_does_not_exist.ts', importer);
		} catch (e) {
			error = e;
		}
		assert(error instanceof Error);
		expect((error as NodeJS.ErrnoException).code).toBe('ERR_MODULE_NOT_FOUND');
	});

	test('refuses a js specifier for a missing file without mapping it to its ts sibling', async () => {
		await assert_refused(
			'./test_ts.js',
			"no such file, but './test_ts.ts' exists; write the real extension (.js never maps to .ts)"
		);
		await assert_refused(
			'./test_ts.js?raw',
			"no such file, but './test_ts.ts' exists; write the real extension (.js never maps to .ts)"
		);
	});

	test('refuses a js specifier for a missing file with no sibling', async () => {
		await assert_refused('./test_missing.js', 'no such file');
	});

	test('refuses an extensionless specifier, naming the file it likely meant', async () => {
		await assert_refused(
			'./test_ts',
			"no such file, but './test_ts.ts' exists; write the real extension (nothing is probed)"
		);
		await assert_refused(
			'./test_js',
			"no such file, but './test_js.js' exists; write the real extension (nothing is probed)"
		);
	});

	test('refuses an extensionless specifier for a file that does not exist', async () => {
		await assert_refused(
			'./test_missing',
			'no such file; the path has no extension and nothing is probed, so write the real one'
		);
	});

	test('refuses a file named with a trailing slash', async () => {
		await assert_refused('./test_ts.ts/', 'no such file');
		await assert_refused('./test_ts.ts/.', 'no such file');
	});

	test("refuses a leading '//', which Node reads as a URL host", async () => {
		const specifier = '/' + join(dir, 'test_ts.ts');
		const expected = "a leading '//' names a URL host, not a path; write a single '/'";
		await assert_refused(specifier, expected);
		await assert_refused(specifier + '?raw', expected);
		let error;
		try {
			await resolve_specifier(specifier, importer);
		} catch (e) {
			error = e;
		}
		assert(error instanceof Error);
		expect((error as NodeJS.ErrnoException).code).toBe('ERR_MODULE_NOT_FOUND');
	});

	test('refuses a directory', async () => {
		await assert_refused(
			'./modules',
			'it is not a regular file (a directory or a special file); name a module file'
		);
	});
});

describe('to_specifier_path_id', () => {
	const importer = '/p/src/lib/a.ts';

	test("joins a relative specifier to the importer's directory", () => {
		expect(to_specifier_path_id('./b.ts', importer)).toBe('/p/src/lib/b.ts');
		expect(to_specifier_path_id('../c/d.ts', importer)).toBe('/p/src/c/d.ts');
	});

	test('strips `?raw`', () => {
		expect(to_specifier_path_id('./b.svg?raw', importer)).toBe('/p/src/lib/b.svg');
	});

	test('normalizes an absolute specifier', () => {
		expect(to_specifier_path_id('/p/src/x/../b.ts', importer)).toBe('/p/src/b.ts');
		expect(to_specifier_path_id('../../', importer)).toBe('/p/');
	});

	test('names a path whether or not it exists', () => {
		expect(to_specifier_path_id('./missing.ts', importer)).toBe('/p/src/lib/missing.ts');
	});
});
