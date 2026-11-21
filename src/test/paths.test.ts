import { describe, test, expect } from 'vitest';
import { resolve, join } from 'node:path';

import { create_paths, print_path, to_root_path, to_search_filters_relative } from '#lib/paths.ts';
import { SEARCH_EXCLUDER_DEFAULT } from '#lib/gro_config.ts';
import { paths } from './test_helpers.ts';

describe('create_paths', () => {
	test('basic behavior', () => {
		const root = resolve('../fake');
		const p = create_paths(root);
		expect(p.root).toBe(join(root, '/'));
		expect(p.source).toBe(join(root, 'src/'));
	});
});

describe('to_root_path', () => {
	test('basic behavior', () => {
		expect(to_root_path(resolve('foo/bar'), paths.root)).toBe('foo/bar');
		expect(to_root_path(resolve('./'), paths.root)).toBe('./');
	});

	test('reads a path against the given root', () => {
		expect(to_root_path('/p/src/a.ts', '/p')).toBe('src/a.ts');
		expect(to_root_path('/p/src/a.ts', '/p/')).toBe('src/a.ts');
		expect(to_root_path('src/a.ts', '/p')).toBe('src/a.ts');
		expect(to_root_path('./src/../a.ts', '/p')).toBe('a.ts');
		expect(to_root_path('/q/a.ts', '/p')).toBe('../q/a.ts');
		expect(to_root_path('/p', '/p')).toBe('./');
	});

	test("reads a path in Gro's own package against the given root, as any other", () => {
		expect(to_root_path(paths.source + 'lib/x.ts', '/p')).toBe('..' + paths.source + 'lib/x.ts');
	});
});

describe('print_path', () => {
	test('prints the root path starting with `.`', () => {
		expect(print_path('/p/src/a.ts', '/p')).toBe('./src/a.ts');
		expect(print_path('/q/a.ts', '/p')).toBe('../q/a.ts');
		expect(print_path('/p', '/p')).toBe('./');
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

	test('tests a path outside the root relative to a search dir outside the root', () => {
		const [filter] = to_search_filters_relative(
			(p) => !SEARCH_EXCLUDER_DEFAULT.test(p),
			'/a/c',
			'/x/.cache/other/src'
		);
		expect(filter!('/x/.cache/other/src/lib/x.ts', false)).toBe(true);
		expect(filter!('/x/.cache/other/src/.hidden', true)).toBe(false);
		expect(filter!('/x/.cache/other/src/node_modules', true)).toBe(false);
		expect(filter!('/a/c/src/x.ts', false)).toBe(true);
		expect(filter!('/a/c/.hidden', true)).toBe(false);
		expect(filter!('/x/.cache/elsewhere/x.ts', false)).toBe(false); // outside both, absolute
	});

	test('tests a path under the root relative to the root, even searching above it', () => {
		const received: Array<string> = [];
		const [filter] = to_search_filters_relative(
			(p) => {
				received.push(p);
				return true;
			},
			'/a/c',
			'/a'
		);
		filter!('/a/c/src/x.ts', false);
		filter!('/a/d/x.ts', false);
		filter!('/b/x.ts', false);
		expect(received).toEqual(['src/x.ts', 'd/x.ts', '/b/x.ts']);
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
