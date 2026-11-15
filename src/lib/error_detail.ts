/**
 * Printing an error with what explains it: its `message` and its chain of causes.
 *
 * A host module's error says only `kind: target` in its `message`, and what the OS or the
 * module said of the failure is its `cause`, so code that shows a failure to a person prints
 * the chain, as Gro's own reports do. Grants nothing and imports nothing, so any module,
 * a package's included, prints errors with it.
 *
 * @module
 */

/**
 * The most causes printed beneath an error; a longer chain ends with `…`.
 */
export const MAX_ERROR_CAUSES = 8;

/**
 * The message of `error` followed by its causes', each in parentheses inside the one before:
 * `not_found: /p/a.txt (ENOENT: no such file or directory, open '/p/a.txt')`.
 * An `Error`'s message and a string are printed; the chain ends at any other value, at a cause
 * already printed, and past `MAX_ERROR_CAUSES`. A thrown value that isn't an `Error` prints as
 * `String` prints a primitive, and an object as its type in brackets (`[object]`). Control and bidirectional formatting characters are
 * written as `\u{…}` escapes, so text the error's maker chose prints as one line that can't move
 * the cursor or reorder what a terminal shows. Never throws: a getter that throws, or a proxy,
 * ends the chain there.
 * @param error - any thrown value, typically a host module's error
 */
export const format_error_detail = (error: unknown): string => {
	const head = to_text(error) ?? to_primitive_text(error);
	const causes = format_error_causes(error);
	return causes === null ? head : `${head} (${causes})`;
};

/**
 * The chain of causes beneath `error`, printed as `format_error_detail` prints them,
 * without `error`'s own message, for a report that has already shown it, such as a stack:
 * `ENOENT: no such file or directory, open '/p/a.txt'`. `null` when there is no cause to print.
 * Never throws.
 * @param error - any thrown value
 */
export const format_error_causes = (error: unknown): string | null => {
	const seen = new Set<unknown>([error]);
	const printed: Array<string> = [];
	let cause = read_cause(error);
	while (cause !== undefined && !seen.has(cause)) {
		if (printed.length === MAX_ERROR_CAUSES) {
			printed.push('…');
			break;
		}
		const text = to_text(cause);
		if (text === null) break;
		if (text !== '') printed.push(text);
		seen.add(cause);
		cause = read_cause(cause);
	}
	if (printed.length === 0) return null;
	let joined = printed[printed.length - 1]!;
	for (let i = printed.length - 2; i >= 0; i--) joined = `${printed[i]} (${joined})`;
	return joined;
};

// an `Error`'s message or a string, escaped, else `null`; never throws
const to_text = (value: unknown): string | null => {
	try {
		if (typeof value === 'string') return escape_controls(value);
		if (value instanceof Error) {
			const { message } = value;
			return typeof message === 'string' ? escape_controls(message) : null;
		}
	} catch {
		// a proxy's trap or a getter threw: nothing to print
	}
	return null;
};

// a primitive as `String` prints it, escaped, and an object or function as its type in brackets
const to_primitive_text = (value: unknown): string => {
	switch (typeof value) {
		case 'object':
			return value === null ? '[null]' : '[object]';
		case 'function':
			return '[function]';
		case 'symbol':
			// `String` reads no `Symbol.prototype.toString`, which code can patch to throw
			return escape_controls(String(value));
		default:
			return escape_controls(`${value as string | number | bigint | boolean | undefined}`);
	}
};

// an `Error`'s `cause`, or `undefined` for anything else and for a read that throws
const read_cause = (value: unknown): unknown => {
	try {
		return value instanceof Error ? value.cause : undefined;
	} catch {
		return undefined;
	}
};

// control characters (C0, DEL, C1) and the bidirectional formatting characters
const is_escaped = (c: number): boolean =>
	c < 0x20 ||
	(c >= 0x7f && c <= 0x9f) ||
	c === 0x61c ||
	c === 0x200e ||
	c === 0x200f ||
	(c >= 0x202a && c <= 0x202e) ||
	(c >= 0x2066 && c <= 0x2069);

const escape_controls = (text: string): string => {
	let escaped = '';
	let start = 0;
	for (let i = 0; i < text.length; i++) {
		const c = text.charCodeAt(i);
		if (!is_escaped(c)) continue;
		escaped += text.slice(start, i) + '\\u{' + c.toString(16) + '}';
		start = i + 1;
	}
	return start === 0 ? text : escaped + text.slice(start);
};
