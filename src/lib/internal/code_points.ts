/**
 * Comparing strings by code point, the order of their UTF-8 bytes (JS's default sort compares
 * UTF-16 code units, and `localeCompare` follows a locale), and `/`-joined paths segment by
 * segment. Imports nothing.
 *
 * @module
 */

/**
 * Compares `a` and `b` by code point, which is the order of their UTF-8 bytes, for `sort`.
 * UTF-16 code units agree with it but for a surrogate (U+D800 to U+DFFF), which sorts below
 * U+E000 to U+FFFF as a code unit, while the code point it encodes sorts above them.
 * @returns a negative number when `a` comes first, a positive one when `b` does, else `0`
 */
export const compare_code_points = (a: string, b: string): number => {
	const length = a.length < b.length ? a.length : b.length;
	for (let i = 0; i < length; i++) {
		const c = a.charCodeAt(i);
		const d = b.charCodeAt(i);
		if (c !== d) return compare_differing_units(c, d);
	}
	return a.length - b.length;
};

/**
 * Compares `/`-joined paths segment by segment, each segment by code point, a path before
 * any path beneath it, in one pass over the two strings, splitting neither.
 *
 * It's `compare_code_points` with `/` ranked below every other code unit. Up to the first
 * code unit where the paths differ they share every segment and the start of the one that
 * differs. If one of the two units is the `/`, that path's segment ends there while the other's
 * goes on with the same start, so it's the shorter segment and sorts first by either reading;
 * otherwise both units sit at the same place in the same segment, and both readings compare
 * them by code point. When one path is a prefix of the other, the shorter sorts first by
 * either reading too. So the order equals splitting on `/` and comparing the segments, for any
 * two strings, including empty segments and characters below `/` (space, `!`, `-`, `.`),
 * where it differs from `compare_code_points` over the whole paths: `a/z` comes before `a-b`.
 * The `/` must rank strictly below: mapping it to NUL would tie `a\0b` with `a/b`.
 * @returns a negative number when `a` comes first, a positive one when `b` does, else `0`
 */
export const compare_paths_by_segment = (a: string, b: string): number => {
	const length = a.length < b.length ? a.length : b.length;
	for (let i = 0; i < length; i++) {
		const c = a.charCodeAt(i);
		const d = b.charCodeAt(i);
		if (c === d) continue;
		if (c === 0x2f) return -1;
		if (d === 0x2f) return 1;
		return compare_differing_units(c, d);
	}
	return a.length - b.length;
};

// two differing UTF-16 code units in code point order: the surrogates above U+E000 to U+FFFF,
// each range keeping its own order
const compare_differing_units = (c: number, d: number): number =>
	c >= 0xd800 && d >= 0xd800
		? (c < 0xe000 ? c + 0x2000 : c - 0x800) - (d < 0xe000 ? d + 0x2000 : d - 0x800)
		: c - d;
