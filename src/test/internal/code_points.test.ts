import { test, assert } from 'vitest';

import { compare_code_points, compare_paths_by_segment } from '#lib/internal/code_points.ts';

// a code point from each range whose UTF-16 order differs from its UTF-8 order, and around them
const POINTS = [
	0x0, 0x2f, 0x7f, 0xe9, 0x7ff, 0xd7ff, 0xe000, 0xfb01, 0xfffd, 0xffff, 0x10000, 0x1f600, 0x10ffff
];

const compare_bytes = (a: string, b: string): number =>
	Buffer.compare(Buffer.from(a), Buffer.from(b));

test("orders as the strings' UTF-8 bytes do, where UTF-16 code units disagree", () => {
	const strings: Array<string> = [''];
	for (const x of POINTS) {
		strings.push(String.fromCodePoint(x));
		for (const y of POINTS) strings.push(String.fromCodePoint(x, y));
	}
	for (const a of strings) {
		for (const b of strings) {
			assert.strictEqual(
				Math.sign(compare_code_points(a, b)),
				Math.sign(compare_bytes(a, b)),
				`${JSON.stringify(a)} ${JSON.stringify(b)}`
			);
		}
	}
	// the case UTF-16 gets wrong: an astral point above U+E000 to U+FFFF
	assert.ok(compare_code_points('\u{1F600}', '\uE000') > 0);
	assert.ok('\u{1F600}' < '\uE000');
});

// segment by segment, each segment by its UTF-8 bytes, a path before any path beneath it:
// a reference sharing nothing with the comparator but the order it should give
const to_segment_bytes = (path: string): Array<Buffer> =>
	path.split('/').map((segment) => Buffer.from(segment));
const compare_segment_bytes = (x: Array<Buffer>, y: Array<Buffer>): number => {
	const length = x.length < y.length ? x.length : y.length;
	for (let i = 0; i < length; i++) {
		const compared = Buffer.compare(x[i]!, y[i]!);
		if (compared !== 0) return compared;
	}
	return x.length - y.length;
};

// the separator, characters below it, around it, and the code points whose UTF-16 order differs
const PATH_PIECES = [
	'/',
	' ',
	'!',
	'-',
	'.',
	'0',
	'a',
	'b',
	'\u00E9',
	'\uE000',
	'\uFFFD',
	'\u{1F600}',
	'\u{10FFFF}'
];

// prefix segments: a name, a name beneath it, and its siblings with a character below `/`
const PATHS = ['a', 'a/b', 'a-b', 'a.b', 'a b', 'a!', 'a/b/c', 'a/b-c', 'a/b.c', 'a-b/c', 'ab'];

test('`compare_paths_by_segment` orders as splitting on `/` and comparing segments does, in both argument orders', () => {
	const strings: Array<string> = ['', ...PATHS];
	for (const x of PATH_PIECES) {
		strings.push(x);
		for (const y of PATH_PIECES) strings.push(x + y);
		for (const path of PATHS) strings.push(path + x, x + path, path + '/' + x);
	}
	const segments = strings.map(to_segment_bytes);
	const mismatches: Array<string> = [];
	for (let i = 0; i < strings.length; i++) {
		const a = strings[i]!;
		for (let j = 0; j < strings.length; j++) {
			const b = strings[j]!;
			const expected = Math.sign(compare_segment_bytes(segments[i]!, segments[j]!));
			if (
				Math.sign(compare_paths_by_segment(a, b)) !== expected ||
				Math.sign(compare_paths_by_segment(b, a)) !== -expected
			) {
				mismatches.push(`${JSON.stringify(a)} ${JSON.stringify(b)}`);
			}
		}
	}
	assert.deepEqual(mismatches, []);
});

test("`compare_paths_by_segment` puts a directory's paths where its name sorts", () => {
	const paths = ['a.ts', 'a-b.ts', 'a/z.ts', 'a/\u{1F600}.ts', 'a/\uE000.ts', 'a b.ts', 'a'];
	assert.deepEqual(paths.sort(compare_paths_by_segment), [
		'a',
		'a/z.ts',
		'a/\uE000.ts',
		'a/\u{1F600}.ts',
		'a b.ts',
		'a-b.ts',
		'a.ts'
	]);
	// a whole path's code points put the `/` (0x2F) above the `-` (0x2D) and `.` (0x2E)
	assert.ok(compare_code_points('a/z.ts', 'a-b.ts') > 0);
	assert.ok(compare_paths_by_segment('a/z.ts', 'a-b.ts') < 0);
	// the `/` ranks strictly below every character, NUL included, so they never tie
	assert.ok(compare_paths_by_segment('a/b', 'a\0b') < 0);
	assert.ok(compare_paths_by_segment('a\0b', 'a/b') > 0);
	assert.strictEqual(compare_paths_by_segment('a/b', 'a/b'), 0);
});
