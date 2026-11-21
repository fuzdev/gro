/**
 * Glob patterns matched against `/`-separated paths as text, as `node:path`'s `matchesGlob`
 * matches them on POSIX, for the modules that load as a package on the runtime,
 * where `node:path` doesn't. The syntax is the common one: `*`, `?`, `[...]`, `**` as a
 * whole segment, and flat `{a,b}` alternatives. Extended globs (`@(…)`, `!(…)`, `+(…)`,
 * `*(…)`, `?(…)`), brace sequences (`{1..3}`), a brace group inside another (`{a,{b,c}}`),
 * unbalanced braces, `${`, POSIX classes (`[[:alpha:]]`), a class range out of order
 * (`[z-a]`), and a `..` segment are refused, never matched as literal text or read another
 * way, so a pattern never silently matches less.
 *
 * @module
 */

import { TaskError } from '../task_error.ts';

/**
 * How a glob compiles.
 */
export interface GlobOptions {
	/**
	 * The pattern as the user wrote it, which a refusal names, defaults to `pattern`.
	 */
	written?: string;
}

/**
 * Matches a normalized path, with no `.`, `..`, or empty segment, as an fs search's ids are,
 * against a compiled glob.
 */
export type GlobMatcher = (path: string) => boolean;

const GLOBSTAR = Symbol('globstar');

type Segment = RegExp | string | typeof GLOBSTAR;

const EXTGLOB_MATCHER = /[@!+*?]\(/;
const BRACE_SEQUENCE_MATCHER = /^(-?\d+\.\.-?\d+(\.\.-?\d+)?|[a-zA-Z]\.\.[a-zA-Z](\.\.-?\d+)?)$/;
const REGEXP_SPECIAL_MATCHER = /[\\^$.*+?()[\]{}|/]/g;

/**
 * Compiles a glob, `matchesGlob`'s reading of it on POSIX: a backslash is a path separator,
 * never an escape; a wildcard never matches a `/` or a segment's leading `.`;
 * and `**` as a whole segment matches any number of segments, none starting with `.`.
 * @throws `TaskError` - for syntax it doesn't support, extended globs, brace sequences,
 * a brace group inside another, unbalanced braces, `${`, POSIX classes, a class range out of
 * order, and a `..` segment, naming the pattern as written
 */
export const glob_to_matcher = (pattern: string, options: GlobOptions = {}): GlobMatcher => {
	const { written = pattern } = options;
	const normalized = pattern.replace(/\\/g, '/');
	// the shell keeps `${…}` as written, as `matchesGlob` does, which the expansion doesn't follow
	if (normalized.includes('${')) throw unsupported(written, 'a `${` brace');
	const brace_fault = to_brace_fault(normalized);
	if (brace_fault !== null) {
		throw unsupported(
			written,
			brace_fault.indexes.some((index) => is_in_class(normalized, index))
				? IN_CLASS_FAULT
				: brace_fault.what
		);
	}
	const expanded = expand_braces(normalized, written);
	// a `..` left after the backslashes become separators or the braces expand,
	// as `{,a}` does after one, which `matchesGlob` cancels
	if (expanded.some((p) => p.split('/').includes('..'))) {
		throw unsupported(written, 'a `..` segment');
	}
	// checked after the expansion, which can make one, as `{,a}(` does
	for (const p of expanded) {
		const extglob = EXTGLOB_MATCHER.exec(p);
		if (extglob === null) continue;
		throw unsupported(
			written,
			is_in_class(p, extglob.index + 1) ? IN_CLASS_FAULT : 'an extended glob'
		);
	}
	const compiled = expanded.map((p) => to_segments(p, written));
	return (path) => {
		const segments = path.split('/');
		return compiled.some((pattern_segments) => match_segments(pattern_segments, segments));
	};
};

const unsupported = (pattern: string, what: string): TaskError =>
	new TaskError(`the glob \`${pattern}\` uses ${what}, which Gro's globs don't support`);

// what a refusal names when the brace or `(` it refuses is a character class's,
// like `[{]a` or `[!(]a`, which Node's `matchesGlob` matches as a class
const IN_CLASS_FAULT = 'a brace or `(` inside a character class';

/**
 * Whether the character at `index` is inside one of the pattern's character classes,
 * a `[` to the next `]` in its segment, a `]` right after the `[` or its `!` or `^` a member.
 */
const is_in_class = (pattern: string, index: number): boolean => {
	for (const match of pattern.matchAll(/\[[!^]?\]?[^\]/]*\]/g)) {
		if (index > match.index && index < match.index + match[0].length - 1) return true;
	}
	return false;
};

/**
 * The patterns the first brace group's `{a,b}` alternatives expand to, each expanded in turn,
 * a group with no comma kept as literal text, as brace expansion keeps it.
 * Every group is flat, `to_brace_fault` having refused one inside another.
 */
const expand_braces = (pattern: string, written: string): Array<string> => {
	const open = pattern.indexOf('{');
	if (open === -1) return [pattern];
	const close = pattern.indexOf('}', open);
	const before = pattern.slice(0, open);
	const body = pattern.slice(open + 1, close);
	const rest = expand_braces(pattern.slice(close + 1), written);
	if (!body.includes(',')) {
		if (BRACE_SEQUENCE_MATCHER.test(body)) throw unsupported(written, 'a brace sequence');
		return rest.map((after) => before + '{' + body + '}' + after);
	}
	return body
		.split(',')
		.flatMap((alternative) => rest.map((after) => before + alternative + after));
};

/**
 * What's wrong with a pattern's braces for a flat expansion, and the indexes of the braces at
 * fault, the one that broke the pairing and the one that opened or last closed a group before it,
 * so a refusal can tell a class's brace from a group's, or `null` when nothing is:
 * a group inside another, whose expansion `matchesGlob` reads in ways a flat one never does,
 * or braces that don't pair, a `}` before its `{` or a `{` never closed.
 */
const to_brace_fault = (pattern: string): { what: string; indexes: Array<number> } | null => {
	let open = -1;
	let closed = -1;
	for (let i = 0; i < pattern.length; i++) {
		const char = pattern[i];
		if (char === '{') {
			if (open !== -1) return { what: 'a brace group inside another', indexes: [open, i] };
			open = i;
		} else if (char === '}') {
			if (open === -1) {
				return { what: 'unbalanced braces', indexes: closed === -1 ? [i] : [closed, i] };
			}
			open = -1;
			closed = i;
		}
	}
	return open === -1 ? null : { what: 'unbalanced braces', indexes: [open] };
};

/**
 * The segments of a pattern, an inner `.` segment and a repeated `/` dropped,
 * as `matchesGlob` reads it, the first and last segments kept.
 */
const to_path_segments = (path: string): Array<string> =>
	path
		.split('/')
		.filter(
			(segment, i, all) => i === 0 || i === all.length - 1 || (segment !== '.' && segment !== '')
		);

const to_segments = (pattern: string, written: string): Array<Segment> =>
	to_path_segments(pattern).map((segment): Segment => {
		if (segment === '**') return GLOBSTAR;
		const source = to_segment_source(segment, written);
		if (source === null) return segment;
		try {
			return new RegExp('^' + source + '$');
		} catch {
			// a class's range out of order, which `matchesGlob` reads inconsistently,
			// matching nothing for `[z-a]` but the class's other members for `[z-ab]`
			throw unsupported(written, 'a character class whose range is out of order');
		}
	});

/**
 * The regexp source of one segment, `null` when it holds no glob syntax and matches as text.
 */
const to_segment_source = (segment: string, pattern: string): string | null => {
	let source = '';
	let magic = false;
	// whether the segment's first character is a literal `.`, as `[.]` is
	let leading_dot = segment.startsWith('.');
	for (let i = 0; i < segment.length; i++) {
		const char = segment[i]!;
		if (char === '*') {
			// a run of stars inside a segment is one star
			while (segment[i + 1] === '*') i++;
			source += '[^/]*';
			magic = true;
		} else if (char === '?') {
			source += '[^/]';
			magic = true;
		} else if (char === '[') {
			const close = find_class_end(segment, i);
			if (close === -1) {
				source += '\\[';
				continue;
			}
			let body = segment.slice(i + 1, close);
			if (body.includes('[:')) throw unsupported(pattern, 'a POSIX class');
			// a class of one character is that character, as `matchesGlob` reads it
			if (body.length === 1) {
				if (i === 0 && body === '.') leading_dot = true;
				source += body.replace(REGEXP_SPECIAL_MATCHER, '\\$&');
				i = close;
				continue;
			}
			let negated = false;
			if (body[0] === '!' || body[0] === '^') {
				negated = true;
				body = body.slice(1);
			}
			source += (negated ? '[^/' : '[') + body.replace(/[\\\]^]/g, '\\$&') + ']';
			magic = true;
			i = close;
		} else {
			source += char.replace(REGEXP_SPECIAL_MATCHER, '\\$&');
		}
	}
	// a class of one character makes the segment a pattern with no wildcard
	if (!magic) return segment.includes('[') ? source : null;
	// a wildcard never matches a leading `.`, so only a pattern starting with one does
	return leading_dot ? source : '(?!\\.)' + source;
};

// the index of the `]` closing the class opened at `open`, a `]` first in it being literal
const find_class_end = (segment: string, open: number): number => {
	let i = open + 1;
	if (segment[i] === '!' || segment[i] === '^') i++;
	if (segment[i] === ']') i++;
	for (; i < segment.length; i++) {
		if (segment[i] === ']') return i;
	}
	return -1;
};

const match_segments = (pattern: Array<Segment>, path: Array<string>): boolean => {
	// the pattern and path positions that failed, so repeated globstars stay linear per pair
	const failed: Set<number> = new Set();
	const width = path.length + 1;
	const match = (p: number, s: number): boolean => {
		if (failed.has(p * width + s)) return false;
		let matched: boolean;
		if (p === pattern.length) {
			matched = s === path.length;
		} else {
			const segment = pattern[p]!;
			if (segment === GLOBSTAR) {
				// a trailing one matches at least one segment, so `a/**` doesn't match `a`
				matched =
					(p + 1 < pattern.length && match(p + 1, s)) ||
					(s < path.length &&
						!path[s]!.startsWith('.') &&
						(match(p + 1, s + 1) || match(p, s + 1)));
			} else {
				const name = path[s];
				matched =
					name !== undefined &&
					// a wildcard never matches `.` or `..`
					(typeof segment === 'string'
						? segment === name
						: name !== '.' && name !== '..' && segment.test(name)) &&
					match(p + 1, s + 1);
			}
		}
		if (!matched) failed.add(p * width + s);
		return matched;
	};
	return match(0, 0);
};
