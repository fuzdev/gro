import { test, assert } from 'vitest';
import { posix } from 'node:path';

import {
	posix_basename,
	posix_dirname,
	posix_join,
	posix_normalize,
	posix_resolve
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
