/**
 * The working directory, checked to be the directory the process is in.
 *
 * Node decodes `getcwd` as UTF-8 lossily, so in a directory whose path isn't UTF-8,
 * `process.cwd()` names a different path, one with U+FFFD where the bytes were,
 * and everything resolved against it would reach that other directory.
 *
 * @module
 */

import { realpathSync } from 'node:fs';

// the libc call on `.` itself, where the JS `realpathSync` resolves `.` against `process.cwd()`
const realpath_native = realpathSync.native;

// fatal, so a path that isn't UTF-8 throws rather than decoding to U+FFFD, BOM kept
const utf8_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/**
 * The working directory's real path, read as bytes, one syscall.
 * @throws `Error` - when the path isn't UTF-8, or isn't what `process.cwd()` reports
 */
export const to_checked_cwd = (): string => {
	const cwd = process.cwd();
	let real: string;
	try {
		real = utf8_decoder.decode(realpath_native('.', { encoding: 'buffer' }));
	} catch (error) {
		if (error instanceof TypeError) {
			throw new Error(
				`the working directory's path isn't UTF-8, so Gro can't name it (Node reports ${cwd})`,
				{ cause: error }
			);
		}
		throw error;
	}
	if (real !== cwd) {
		throw new Error(`the working directory ${real} isn't what Node reports, ${cwd}`);
	}
	return real;
};
