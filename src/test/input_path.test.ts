import { test, expect } from 'vitest';
import { resolve } from 'node:path';
import type { ResolvedPath } from '@fuzdev/fuz_util/path.ts';

import {
	to_input_path,
	to_input_paths,
	resolve_input_files,
	get_possible_paths,
	type ResolvedInputPath,
	type ResolvedInputFile
} from '$lib/input_path.ts';
import { GRO_DIST_DIR, paths } from '$lib/paths.ts';

test('to_input_path', () => {
	expect(to_input_path(resolve('foo.ts'))).toBe(resolve('foo.ts'));
	expect(to_input_path('./foo.ts')).toBe(resolve('foo.ts'));
	expect(to_input_path('foo.ts')).toBe('foo.ts');
	expect(to_input_path('gro/foo')).toBe(GRO_DIST_DIR + 'foo');
	// trailing slashes are preserved:
	expect(to_input_path(resolve('foo/bar/'))).toBe(resolve('foo/bar/'));
	expect(to_input_path('./foo/bar/')).toBe(resolve('foo/bar/'));
	expect(to_input_path('foo/bar/')).toBe('foo/bar/');
});

test('to_input_paths', () => {
	expect(to_input_paths([resolve('foo/bar.ts'), './baz', 'foo'])).toEqual([
		resolve('foo/bar.ts'),
		resolve('baz'),
		'foo'
	]);
});

test('get_possible_paths with an implicit relative path', async () => {
	const input_path = 'src/foo/bar';
	expect(
		await get_possible_paths(
			input_path,
			[resolve('src/foo'), resolve('src/baz'), resolve('src'), resolve('.')],
			['.ext.ts']
		)
	).toEqual([
		{
			id: resolve('src/foo/src/foo/bar'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src/foo')
		},
		{
			id: resolve('src/foo/src/foo/bar.ext.ts'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src/foo')
		},
		{
			id: resolve('src/baz/src/foo/bar'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src/baz')
		},
		{
			id: resolve('src/baz/src/foo/bar.ext.ts'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src/baz')
		},
		{
			id: resolve('src/src/foo/bar'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src')
		},
		{
			id: resolve('src/src/foo/bar.ext.ts'),
			input_path: 'src/foo/bar',
			root_dir: resolve('src')
		},
		{
			id: resolve('src/foo/bar'),
			input_path: 'src/foo/bar',
			root_dir: resolve('.')
		},
		{
			id: resolve('src/foo/bar.ext.ts'),
			input_path: 'src/foo/bar',
			root_dir: resolve('.')
		}
	]);
});

test('get_possible_paths in the gro directory', async () => {
	const input_path = resolve('src/foo/bar');
	expect(await get_possible_paths(input_path, [], ['.ext.ts'])).toEqual([
		{ id: input_path, input_path: resolve('src/foo/bar'), root_dir: resolve('src/foo') },
		{ id: input_path + '.ext.ts', input_path: resolve('src/foo/bar'), root_dir: resolve('src/foo') }
	]);
});

test('get_possible_paths does not repeat the extension', async () => {
	const input_path = resolve('src/foo/bar.ext.ts');
	expect(await get_possible_paths(input_path, [], ['.ext.ts'])).toEqual([
		{ id: input_path, input_path: resolve('src/foo/bar.ext.ts'), root_dir: resolve('src/foo') }
	]);
});

test('get_possible_paths does not repeat with the same root directory', async () => {
	const input_path = resolve('src/foo/bar.ext.ts');
	expect(await get_possible_paths(input_path, [paths.root, paths.root], ['.ext.ts'])).toEqual([
		{ id: input_path, input_path: resolve('src/foo/bar.ext.ts'), root_dir: resolve('src/foo') }
	]);
});

test('get_possible_paths implied to be a directory by trailing slash', async () => {
	const input_path = resolve('src/foo/bar') + '/';
	expect(await get_possible_paths(input_path, [], ['.ext.ts'])).toEqual([
		{ id: input_path, input_path: resolve('src/foo/bar') + '/', root_dir: resolve('src/foo') }
	]);
});

test('resolve_input_files', async () => {
	const test_files: Record<string, Array<ResolvedPath>> = {
		'fake/test1.ext.ts': [
			{ id: 'fake/test1.ext.ts', path: 'fake/test1.ext.ts', is_directory: false }
		],
		'fake/test2.ext.ts': [
			{ id: 'fake/test2.ext.ts', path: 'fake/test2.ext.ts', is_directory: false }
		],
		'fake/test3': [
			{ id: 'fake/test3', path: 'fake/test3', is_directory: true },
			{ id: 'a.ts', path: 'a.ts', is_directory: false },
			{ id: 'b.ts', path: 'b.ts', is_directory: false }
		],
		// duplicate
		'fake/': [
			{ id: 'fake/test3', path: 'fake/test3', is_directory: true },
			{ id: 'test3/a.ts', path: 'test3/a.ts', is_directory: false }
		],
		// duplicate and not
		fake: [
			{ id: 'fake/test3', path: 'fake/test3', is_directory: true },
			{ id: 'test3/a.ts', path: 'test3/a.ts', is_directory: false },
			{ id: 'test3/c.ts', path: 'test3/c.ts', is_directory: false }
		],
		'fake/nomatches': [{ id: 'fake/nomatches', path: 'fake/nomatches', is_directory: true }],
		fake2: [{ id: 'test.ext.ts', path: 'test.ext.ts', is_directory: false }]
	};
	const a: ResolvedInputPath = {
		id: 'fake/test1.ext.ts',
		is_directory: false,
		input_path: 'fake/test1.ext.ts',
		root_dir: process.cwd()
	};
	const b: ResolvedInputPath = {
		id: 'fake/test2.ext.ts',
		is_directory: false,
		input_path: 'fake/test2',
		root_dir: process.cwd()
	};
	const c: ResolvedInputPath = {
		id: 'fake/test3',
		is_directory: true,
		input_path: 'fake/test3',
		root_dir: process.cwd()
	};
	const d: ResolvedInputPath = {
		id: 'fake',
		is_directory: true,
		input_path: 'fake',
		root_dir: process.cwd()
	};
	const e: ResolvedInputPath = {
		id: 'fake/nomatches',
		is_directory: true,
		input_path: 'fake/nomatches',
		root_dir: process.cwd()
	};
	// These two have the same id from different directory input paths.
	const f: ResolvedInputPath = {
		id: 'fake2',
		is_directory: true,
		input_path: 'fake2',
		root_dir: process.cwd()
	};
	const g: ResolvedInputPath = {
		id: 'fake2',
		is_directory: true,
		input_path: './fake2/',
		root_dir: process.cwd()
	};
	// These two have the same id from different file input paths.
	const h: ResolvedInputPath = {
		id: 'fake3/test.ext.ts',
		is_directory: false,
		input_path: 'fake3/test.ext.ts',
		root_dir: process.cwd()
	};
	const i: ResolvedInputPath = {
		id: 'fake3/test.ext.ts',
		is_directory: false,
		input_path: 'fake3/test',
		root_dir: process.cwd()
	};
	const result = await resolve_input_files([a, b, c, d, e, f, g, h, i], (dir) =>
		Promise.resolve(test_files[dir]!)
	);
	const resolved_input_files: Array<ResolvedInputFile> = [
		{ id: a.id, input_path: a.input_path, resolved_input_path: a },
		{ id: b.id, input_path: b.input_path, resolved_input_path: b },
		{ id: 'fake/test3/a.ts', input_path: c.input_path, resolved_input_path: c },
		{ id: 'fake/test3/b.ts', input_path: c.input_path, resolved_input_path: c },
		{ id: 'fake/test3/c.ts', input_path: d.input_path, resolved_input_path: d },
		{ id: 'fake2/test.ext.ts', input_path: f.input_path, resolved_input_path: f },
		{ id: 'fake3/test.ext.ts', input_path: h.input_path, resolved_input_path: h }
	];
	expect(result).toEqual({
		resolved_input_files,
		resolved_input_files_by_root_dir: new Map([
			[
				process.cwd(),
				[
					{ id: 'fake/test1.ext.ts', input_path: a.input_path, resolved_input_path: a },
					{ id: 'fake/test2.ext.ts', input_path: b.input_path, resolved_input_path: b },
					{ id: 'fake/test3/a.ts', input_path: c.input_path, resolved_input_path: c },
					{ id: 'fake/test3/b.ts', input_path: c.input_path, resolved_input_path: c },
					{ id: 'fake/test3/c.ts', input_path: d.input_path, resolved_input_path: d },
					{ id: 'fake2/test.ext.ts', input_path: f.input_path, resolved_input_path: f },
					{ id: 'fake3/test.ext.ts', input_path: h.input_path, resolved_input_path: h }
				]
			]
		]),
		input_directories_with_no_files: [e.input_path]
	});
});
