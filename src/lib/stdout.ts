/**
 * The stdout host module: the running process's standard output, as an import rather than a
 * global, and `ProcessOutput`, the type it shares with the stderr host module.
 *
 * The handle and its prototype are frozen, null-prototype so it isn't thenable even with a
 * polluted `Object.prototype.then`, and its `constructor` refuses without a key only Gro holds.
 * A method called on any other object throws `TypeError`, private fields being the only state.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can write to `process.stdout`.
 * - On POSIX Node's stdio writes to files and terminals block the event loop until the kernel
 *   takes them, where the runtime's complete on its own schedule. Writes to pipes are asynchronous.
 * - Intrinsics stay pollutable. Node's stream `write` is the stream's own, so code that patched it
 *   runs inside a write, and the stream is created on first use, so one replaced on the global by
 *   then is the one written to. The stream classes and `EventEmitter.prototype` (`once`,
 *   `listenerCount`) stay too, so code that patches them runs inside a write holding the raw stream.
 *
 * @module
 */

import { create_process_output } from './internal/process_output.ts';

/**
 * The process's stdout or stderr.
 */
export interface ProcessOutput {
	/**
	 * Whether this stream is a terminal.
	 */
	readonly is_terminal: boolean;
	/**
	 * Writes `data`, a string as UTF-8, resolving once it's accepted, so awaiting each write
	 * paces a writer against the reader. Writes not awaited queue in memory, and `exit` drops
	 * any not yet accepted, so await a write before exiting.
	 * @returns `false` when the reader has gone (`EPIPE`, a task piped to `head`), so the data
	 * went nowhere, and every later write resolves `false` too
	 * @throws `TypeError` - for data that's neither a well-formed string nor a `Uint8Array`
	 * @throws `StdioError` - `io_error`, any other write failure, like a full disk
	 */
	write(data: string | Uint8Array): Promise<boolean>;
}

/**
 * Stdout.
 */
export const stdout: ProcessOutput = create_process_output('stdout', 1);
