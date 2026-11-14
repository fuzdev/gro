/**
 * The exit host module: control over how the run ends, as an import rather than a global.
 *
 * `exit` ends the process now and `set_exit_code` sets the code it ends with on its own,
 * the outcome whoever started the run sees. Neither flushes a stream: `exit` drops writes
 * to `stdout` and `stderr` not yet accepted, so await a write before exiting.
 *
 * Both are frozen functions taking no receiver, and an exit code is checked before it's used,
 * with `typeof` first, so a thenable or a `Symbol.toPrimitive` code runs no code of the caller's.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can call `process.exit` or set `process.exitCode`.
 * - `exit` in a Node worker ends the thread, not the process.
 *
 * @module
 */

const host_process = globalThis.process;
const process_exit: (code?: number) => never = host_process.exit.bind(host_process);
const { isInteger } = Number;
const { freeze } = Object;

const check_exit_code = (code: unknown): number => {
	// `typeof` first, so a thenable or `Symbol.toPrimitive` code runs no code of the caller's
	if (typeof code !== 'number' || !isInteger(code) || code < 0 || code > 255) {
		throw new TypeError('an exit code is an integer from 0 to 255');
	}
	return code;
};

/**
 * Exits with `code`, or without one the code from `set_exit_code`, else `0`.
 * Spawned children are cleaned up on the way out, by the spawn module's exit hook.
 * Writes not yet accepted are dropped, so await a write before exiting.
 * @param code - an integer from 0 to 255
 * @throws `TypeError` - for any other value, without calling a thenable or a `toPrimitive`
 */
export const exit = (code?: number): never => {
	if (code === undefined) process_exit();
	process_exit(check_exit_code(code));
};

/**
 * Sets the code the process exits with when it next ends on its own.
 * @param code - an integer from 0 to 255
 * @throws `TypeError` - for any other value
 */
export const set_exit_code = (code: number): void => {
	host_process.exitCode = check_exit_code(code);
};

freeze(exit);
freeze(set_exit_code);
