/**
 * Comparing strings by code point, the order of their UTF-8 bytes, where JS's default sort
 * compares UTF-16 code units, and `localeCompare` follows a locale. Imports nothing.
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
		if (c === d) continue;
		if (c >= 0xd800 && d >= 0xd800) {
			// surrogates above U+E000 to U+FFFF, each range keeping its own order
			return (c < 0xe000 ? c + 0x2000 : c - 0x800) - (d < 0xe000 ? d + 0x2000 : d - 0x800);
		}
		return c - d;
	}
	return a.length - b.length;
};
