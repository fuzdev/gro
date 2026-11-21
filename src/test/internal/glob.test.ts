import { test, assert } from 'vitest';
import { matchesGlob } from 'node:path';

import { glob_to_matcher } from '#lib/internal/glob.ts';
import { TaskError } from '#lib/task_error.ts';
import { posix_normalize } from '#lib/internal/posix_path.ts';

/*

`glob_to_matcher` against `node:path`'s `matchesGlob`, whose POSIX reading it reproduces
for normalized paths, the ids an fs search yields.

*/

const PATHS = [
	'/p/src/a.ts',
	'/p/src/b.ts',
	'/p/src/x/a.ts',
	'/p/src/x/y/a.ts',
	'/p/src/.hidden/a.ts',
	'/p/src/.a.ts',
	'/p/src/a.svelte',
	'/p/src/[slug]/+page.ts',
	'/p/src/ab.ts',
	'/p/src/abc.ts',
	'/p/src/a(b).ts',
	'/p/src/{a}.ts',
	'/p/src/a,b.ts',
	'/p/src/A.ts',
	'/p/src',
	'/p/src/x',
	'/p/src/x/.y/a.ts',
	'/p/src/é.ts',
	'/p/src/a b.ts',
	'/p/a.ts',
	'/p/src/x/a.js',
	'/p/src/].ts',
	'/p/src/!a.ts',
	'/p/src/a\\b.ts',
	'/p/src/a$.ts',
	'/p/src/a|b.ts'
];

const PATTERNS = [
	'/p/src/*.ts',
	'/p/src/**/*.ts',
	'/p/src/**',
	'/p/**/a.ts',
	'/p/src/?.ts',
	'/p/src/[ab].ts',
	'/p/src/[!a].ts',
	'/p/src/[^a].ts',
	'/p/src/[a-c]*.ts',
	'/p/src/{a,b}.ts',
	'/p/src/{a}.ts',
	'/p/src/{,a}.ts',
	'/p/src/*/*/a.ts',
	'/p/src/.*.ts',
	'/p/src/**/.hidden/*.ts',
	'/p/src/x/**/a.ts',
	'/p/src/a(b).ts',
	'/p/src/*.{ts,svelte}',
	'/p/src/[]].ts',
	'/p/src/a**b.ts',
	'/p/src/x/**',
	'/p/src/[a',
	'/p/src/[.]a.ts',
	'/p/src/a\\b.ts',
	'/p/src/**/**/a.ts',
	'/p/src/[A-Z].ts',
	'/p/src/{a,b}{,c}.ts',
	'/p/src/./a.ts',
	'/p//src/a.ts'
];

test('glob_to_matcher matches every path as matchesGlob does', () => {
	for (const pattern of PATTERNS) {
		const matcher = glob_to_matcher(pattern);
		for (const path of PATHS) {
			assert.strictEqual(matcher(path), matchesGlob(path, pattern), `${pattern} ${path}`);
		}
	}
});

test('glob_to_matcher agrees with matchesGlob on generated patterns and paths', () => {
	let seed = 0x9e3779b9;
	const next = (n: number): number => {
		seed ^= seed << 13;
		seed ^= seed >>> 17;
		seed ^= seed << 5;
		return (seed >>> 0) % n;
	};
	const pattern_atoms = ['a', 'b', '.', '*', '?', '**', '[ab]', '[!a]', '[.]', '{a,b}', '{,a}'];
	const path_atoms = ['a', 'b', '.', 'x', '/', '-', ',', 'ab', '.a'];
	for (let i = 0; i < 3000; i++) {
		let pattern = '/';
		for (let j = 0, l = 1 + next(6); j < l; j++)
			pattern += pattern_atoms[next(pattern_atoms.length)];
		pattern += '/' + pattern_atoms[next(pattern_atoms.length)];
		// a `..` segment, written or left by the brace expansion, is refused,
		// which the fs search's caller never passes, resolving first
		let matcher;
		try {
			matcher = glob_to_matcher(pattern);
		} catch (error) {
			assert.instanceOf(error, TaskError, pattern);
			assert.include(error.message, '`..`', pattern);
			continue;
		}
		for (let k = 0; k < 10; k++) {
			let path = '/';
			for (let j = 0, l = 1 + next(6); j < l; j++) path += path_atoms[next(path_atoms.length)];
			if (
				path.endsWith('/') ||
				posix_normalize(path) !== path ||
				path.split('/').some((s) => s === '.' || s === '..')
			) {
				continue;
			}
			assert.strictEqual(matcher(path), matchesGlob(path, pattern), `${pattern} ${path}`);
		}
	}
});

test('glob_to_matcher refuses what it does not support, naming the pattern', () => {
	for (const pattern of [
		'/p/+(a|b).ts',
		'/p/@(a).ts',
		'/p/!(a).ts',
		'/p/a*(b).ts',
		'/p/?(a).ts',
		'/p/a**{,a}().ts',
		'/p/{1..3}.ts',
		'/p/{a..c}.ts',
		'/p/[[:alpha:]].ts'
	]) {
		assert.throws(() => glob_to_matcher(pattern), TaskError, pattern);
	}
});

test('glob_to_matcher refuses a class whose range is out of order, which no path matches', () => {
	for (const pattern of ['[z-a].ts', 'src/[b-a]*/x.ts', '{a,[9-0]}']) {
		assert.ok(!matchesGlob('z.ts', pattern));
		assert.throws(() => glob_to_matcher(pattern), TaskError, 'out of order');
	}
});

test('glob_to_matcher refuses braces and a `..` that `matchesGlob` reads otherwise', () => {
	for (const [pattern, path] of [
		// the shell's `${…}` stays as written
		['/p/${a,b}.ts', '/p/${a,b}.ts'],
		// a balanced group inside unbalanced braces expands
		['/p/{{a,b}.ts', '/p/{a.ts'],
		// a group inside another, whose expansion can even make a range of its alternative
		['/p/src/{a,{b,ab}}.ts', '/p/src/ab.ts'],
		['/p/{{a,b}..c}.ts', '/p/c.ts'],
		// a backslash's `..` cancels the segment before it
		['/p/a/..\\*.ts', '/p/x.ts'],
		// as does one an empty alternative leaves
		['/p/a/..{,b}/x.ts', '/p/x.ts']
	] as const) {
		assert.ok(matchesGlob(path, pattern), pattern);
		assert.throws(() => glob_to_matcher(pattern), TaskError, pattern);
	}
	// what each refusal names
	assert.throws(() => glob_to_matcher('/p/{a,{b,c}}'), TaskError, 'a brace group inside another');
	assert.throws(() => glob_to_matcher('/p/{a,b'), TaskError, 'unbalanced braces');
	assert.throws(() => glob_to_matcher('/p/a}{b,c}'), TaskError, 'unbalanced braces');
	assert.throws(() => glob_to_matcher('/p/{1..3}'), TaskError, 'a brace sequence');
	// flat groups, side by side, expand
	assert.ok(glob_to_matcher('/p/{a,b}{,c}.ts')('/p/bc.ts'));
});

test('a refusal names the pattern as written', () => {
	assert.throws(
		() => glob_to_matcher('/p/src/@(a|b).ts', { written: 'src/@(a|b).ts' }),
		TaskError,
		'the glob `src/@(a|b).ts` uses an extended glob'
	);
});

test('a wildcard and a literal segment match case-sensitively, on every OS', () => {
	// Gro's globs never fold case, as `matchesGlob` doesn't on Linux, where the tests run
	const cases: Array<[string, string, boolean]> = [
		['/p/src/*.ts', '/p/src/a.ts', true],
		['/p/src/*.ts', '/p/src/a.TS', false],
		['/p/src/[a-c].ts', '/p/src/B.ts', false],
		['/p/src/?.ts', '/p/src/A.TS', false],
		['/p/*/a.ts', '/p/src/A.ts', false],
		['/p/{src,x}/a.ts', '/p/SRC/a.ts', false],
		['/p/**/a.ts', '/p/X/Y/A.ts', false]
	];
	for (const [pattern, path, expected] of cases) {
		assert.strictEqual(glob_to_matcher(pattern)(path), expected, `${pattern} ${path}`);
		assert.strictEqual(matchesGlob(path, pattern), expected, `Node's on Linux: ${pattern} ${path}`);
	}
});
