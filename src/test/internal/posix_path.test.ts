import { test, assert } from 'vitest';
import { posix } from 'node:path';
import { pathToFileURL } from 'node:url';

import {
	posix_basename,
	posix_dirname,
	posix_extname,
	posix_join,
	posix_normalize,
	posix_relative,
	posix_resolve,
	to_file_url
} from '#lib/internal/posix_path.ts';

/*

`posix_path`'s functions against `node:path`'s POSIX ones, whose readings they reproduce:
the shapes the fs boundary hands them and the edges of the lexical rules.

*/

const PATHS: ReadonlyArray<string> = [
	'',
	'.',
	'./',
	'..',
	'../',
	'/',
	'//',
	'///',
	'///a//b',
	'//a',
	'/..',
	'/../a',
	'/a/../..',
	'/a',
	'/a/',
	'/a//',
	'/a/b',
	'/a/b/',
	'/a/b//',
	'a',
	'a/',
	'a/.',
	'a/..',
	'a/../',
	'a/../..',
	'a/../../b/',
	'./a/./b/',
	'../../a',
	'a/b/../../../c',
	'/home/u/dev/gro/.gro/deploy',
	'/srv/p/./x/../y/',
	'..foo/bar',
	'a/..foo/...',
	'..a',
	'...',
	'.a.b',
	'a.',
	'a..',
	'a.b/',
	'a.b//',
	'/x/a.b.c',
	'/x/.a',
	'/a\0b',
	'/\uD800/x',
	'/�'
];

test('posix_normalize reads every path as posix.normalize does', () => {
	for (const path of PATHS) assert.strictEqual(posix_normalize(path), posix.normalize(path), path);
});

test('posix_dirname reads every path as posix.dirname does', () => {
	for (const path of PATHS) assert.strictEqual(posix_dirname(path), posix.dirname(path), path);
});

test('posix_basename reads every path as posix.basename does', () => {
	for (const path of PATHS) assert.strictEqual(posix_basename(path), posix.basename(path), path);
});

test('posix_join joins as posix.join does, an empty part dropped', () => {
	const parts: ReadonlyArray<ReadonlyArray<string>> = [
		[],
		[''],
		['', ''],
		['/a', 'b'],
		['/a/', 'b/'],
		['/', 'b'],
		['/a', ''],
		['/a', '..'],
		['/a/b', '../../..'],
		['a', '', 'b'],
		['a/', '/b/'],
		...PATHS.map((path) => ['/srv/p', path])
	];
	for (const list of parts) {
		assert.strictEqual(posix_join(...list), posix.join(...list), list.join(' , '));
	}
});

test('posix_resolve resolves against an absolute base as posix.resolve does', () => {
	for (const base of ['/', '//', '/srv/p', '/srv/p/', '/srv//p/.', '/srv/p/..']) {
		for (const path of PATHS) {
			assert.strictEqual(posix_resolve(base, path), posix.resolve(base, path), `${base} ${path}`);
		}
	}
});

test('posix_resolve refuses a relative base, which posix.resolve reads against the cwd', () => {
	for (const base of ['', '.', 'srv/p']) {
		assert.throws(() => posix_resolve(base, 'a'), TypeError, 'absolute base');
	}
});

test('posix_extname reads every path as posix.extname does', () => {
	for (const path of PATHS) assert.strictEqual(posix_extname(path), posix.extname(path), path);
});

test('posix_relative relates absolute paths as posix.relative does', () => {
	const absolute = PATHS.filter((path) => path.startsWith('/'));
	for (const from of absolute) {
		for (const to of absolute) {
			assert.strictEqual(posix_relative(from, to), posix.relative(from, to), `${from} ${to}`);
		}
	}
});

test('posix_relative refuses a relative path, which posix.relative reads against the cwd', () => {
	for (const [from, to] of [
		['a', '/b'],
		['/a', 'b'],
		['', '/b']
	] as const) {
		assert.throws(() => posix_relative(from, to), TypeError, 'absolute paths');
	}
});

test('to_file_url writes the URL of an absolute path as pathToFileURL does', () => {
	const paths = PATHS.filter((path) => path.startsWith('/'));
	// every character of the first two Unicode blocks beyond ASCII, and the shapes around them
	for (let code = 0; code < 0x800; code++) {
		const char = String.fromCharCode(code);
		if (char !== '/') paths.push(`/a${char}b`, `/${char}/`, `/a/${char}${char}.ts`);
	}
	paths.push('/a\u{1f600}b', '/a\ud800b', '/a\udc00', '/a b/../c%20d#e?f/');
	for (const path of paths) {
		assert.strictEqual(to_file_url(path), pathToFileURL(path).href, JSON.stringify(path));
	}
});

test('to_file_url refuses a relative path, which pathToFileURL reads against the cwd', () => {
	assert.throws(() => to_file_url('a/b.ts'), TypeError);
	assert.throws(() => to_file_url(''), TypeError);
});
