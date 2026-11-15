import { test, assert } from 'vitest';

import { compare_code_points } from '#lib/internal/code_points.ts';

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
	assert.ok(compare_code_points('\u{1F600}', '') > 0);
	assert.ok('\u{1F600}' < '');
});
