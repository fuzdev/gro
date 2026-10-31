import { describe, test, expect } from 'vitest';
import { resolve, join } from 'node:path';

import {
	create_paths,
	paths,
	gro_paths,
	is_gro_id,
	to_root_path,
	path_id_to_base_path,
	base_path_to_path_id,
	to_search_filters_relative,
	to_gro_package_dir
} from '#lib/paths.ts';
import { SEARCH_EXCLUDER_DEFAULT } from '#lib/gro_config.ts';

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

describe('to_search_filters_relative', () => {
	const filters = to_search_filters_relative([(p) => !SEARCH_EXCLUDER_DEFAULT.test(p)], '/a/.b/c');

	test('tests the path relative to the root', () => {
		expect(filters[0]!('/a/.b/c/src/x.ts', false)).toBe(true);
		expect(filters[0]!('/a/.b/c/src/.hidden', true)).toBe(false);
		expect(filters[0]!('/a/.b/c/node_modules', true)).toBe(false);
	});

	test('tests a path outside the root as its absolute id', () => {
		expect(filters[0]!('/a/.b/d/x.ts', false)).toBe(false);
		const anchored = to_search_filters_relative((p) => !p.startsWith('src/'), '/a/c');
		expect(anchored[0]!('/a/c/src/x.ts', false)).toBe(false);
		expect(anchored[0]!('/a/c/lib/x.ts', false)).toBe(true);
		expect(anchored[0]!('/a/cc/src/x.ts', false)).toBe(true);
	});

	test('passes the exact relative or absolute path to the filter', () => {
		const received: Array<string> = [];
		const [filter] = to_search_filters_relative((p) => {
			received.push(p);
			return true;
		}, '/a/c');
		filter!('/a/c/src/x.ts', false);
		filter!('/a/d/x.ts', false);
		expect(received).toEqual(['src/x.ts', '/a/d/x.ts']);
	});

	test('accepts one filter or none', () => {
		const filter = (p: string) => p === 'x';
		expect(to_search_filters_relative(filter, '/a/')[0]!('/a/x', false)).toBe(true);
		expect(to_search_filters_relative(null, '/a')).toEqual([]);
	});
});

describe('to_gro_package_dir', () => {
	test('finds the package dir from source or dist under any directory name', () => {
		expect(to_gro_package_dir('/home/me/dev/gro/src/lib/paths.ts')).toBe('/home/me/dev/gro/');
		expect(to_gro_package_dir('/a/.claude/worktrees/wt1/src/lib/paths.ts')).toBe(
			'/a/.claude/worktrees/wt1/'
		);
		expect(to_gro_package_dir('/a/gro_checkout/src/lib/paths.ts')).toBe('/a/gro_checkout/');
		expect(to_gro_package_dir('/a/node_modules/@fuzdev/gro/dist/paths.js')).toBe(
			'/a/node_modules/@fuzdev/gro/'
		);
		expect(to_gro_package_dir('/a/my_fork/dist/paths.js')).toBe('/a/my_fork/');
	});

	test('the gro package dir is this repo when run from source', () => {
		expect(gro_paths.root).toBe(paths.root);
	});
});
