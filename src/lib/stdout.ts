/**
 * The stdout host module: the running process's standard output, as an import rather than a
 * global, and `ProcessOutput`, the type it shares with the stderr host module.
 *
 * The handle and its prototype are frozen, null-prototype so it isn't thenable even with a
 * polluted `Object.prototype.then`, and its `constructor` refuses without a key only Gro holds.
 * Read on any other object `is_terminal` throws `TypeError`, and `write` rejects with it, private
 * fields being the only state.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can write to `process.stdout`.
 * - Node's writes to pipes are asynchronous, queued in memory behind a slow reader, and `exit` drops
 *   any not yet accepted. The runtime writes each one inline, as its console does, taken whole by the
 *   kernel before the call returns, so `exit` drops none and a slow reader stalls the run. On POSIX
 *   Node's writes to files and terminals block until the kernel takes them, as the runtime's do.
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
	 * paces a writer against the reader. On Node, writes not awaited queue in memory, and `exit`
	 * drops any not yet accepted, so await a write before exiting.
	 * An empty write sends nothing and resolves once the writes before it are accepted, so
	 * awaiting one flushes.
	 * @returns `false` when the reader has gone (`EPIPE`, a task piped to `head`), so the data
	 * went nowhere, as does every later write of data; an empty write resolves `true` on the
	 * runtime, which makes no system call for it, and on Node, a write of no bytes, is answered as
	 * the descriptor answers it: `true` on a pipe whose reader has gone, `false` on such a socket,
	 * and `StdioError` on a full disk or a descriptor open only for reading
	 * @throws `TypeError` - for data that's neither a well-formed string nor a `Uint8Array`, or a
	 * call on anything but the handle
	 * @throws `StdioError` - `io_error`, any other write failure, like a full disk
	 */
	write(data: string | Uint8Array): Promise<boolean>;
}

/**
 * Stdout.
 */
export const stdout: ProcessOutput = create_process_output('stdout', 1);
