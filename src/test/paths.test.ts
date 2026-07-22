import { describe, test, expect } from 'vitest';
import { resolve, join } from 'node:path';

import {
	create_paths,
	paths,
	gro_paths,
	is_gro_id,
	to_root_path,
	path_id_to_base_path,
	base_path_to_path_id
} from '$lib/paths.ts';

describe('create_paths', () => {
	test('basic behavior', () => {
		const root = resolve('../fake');
		const p = create_paths(root);
		expect(p.root).toBe(join(root, '/'));
		expect(p.source).toBe(join(root, 'src/'));
	});

	test('paths object has the same identity as the gro_paths object', () => {
		expect(paths).toBe(gro_paths); // because we're testing inside the Gro project
	});
});

describe('is_gro_id', () => {
	test('basic behavior', () => {
		expect(is_gro_id(resolve(paths.root))).toBe(true);
		expect(is_gro_id(resolve(paths.root.slice(0, -1)))).toBe(true);
		expect(is_gro_id(resolve(paths.source).slice(0, -1))).toBe(true);
		expect(is_gro_id(resolve('../fake/src'))).toBe(false);
		expect(is_gro_id(resolve('../fake/src/'))).toBe(false);
		expect(is_gro_id(resolve('../gro_fake'))).toBe(false);
		expect(is_gro_id(resolve('../gro_fake/'))).toBe(false);
		expect(is_gro_id(resolve('../gro_fake/src'))).toBe(false);
		expect(is_gro_id(resolve('../gro_fake/src/'))).toBe(false);
	});
});

describe('to_root_path', () => {
	test('basic behavior', () => {
		expect(to_root_path(resolve('foo/bar'))).toBe('foo/bar');
		expect(to_root_path(resolve('./'))).toBe('./');
		expect(to_root_path(resolve('./'))).toBe('./');
	});
});

describe('path_id_to_base_path', () => {
	test('basic behavior', () => {
		expect(path_id_to_base_path(resolve('src/foo/bar/baz.ts'))).toBe('foo/bar/baz.ts');
	});
});

describe('base_path_to_path_id', () => {
	test('basic behavior', () => {
		expect(base_path_to_path_id('foo/bar/baz.ts')).toBe(resolve('src/foo/bar/baz.ts'));
	});

	test('does not change extension', () => {
		expect(base_path_to_path_id('foo/bar/baz.js')).toBe(resolve('src/foo/bar/baz.js'));
	});
});
