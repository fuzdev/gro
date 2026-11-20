/**
 * How Gro's reports print an error's stack with what explains it: the stack shows only the
 * error's message, so a host module's error, whose message is `kind: target`, is followed by its
 * chain of causes, the OS's or the module's detail.
 *
 * @module
 */

import { print_error, st } from '@fuzdev/fuz_util/print.ts';

import { format_error_causes } from '../error_detail.ts';

/**
 * `text`, an error's stack or message, followed by a line of `error`'s causes when it has any:
 * `cause: ENOENT: no such file or directory, open '/p/a.txt'`.
 * @param text - what's printed of `error` already
 * @param error - any thrown value
 */
export const append_error_causes = (text: string, error: unknown): string => {
	const causes = format_error_causes(error);
	return causes === null ? text : `${text}\ncause: ${causes}`;
};

/**
 * `error`'s stack as `print_error` prints it, followed by its causes, colored alike.
 * @param error - a thrown value that isn't `null` or `undefined`
 */
export const print_error_with_causes = (error: Error): string => {
	const causes = format_error_causes(error);
	return print_error(error) + (causes === null ? '' : st('yellow', `\ncause: ${causes}`));
};
