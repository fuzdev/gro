/**
 * The stderr host module: the running process's standard error, as an import rather than a
 * global. Its type is `ProcessOutput`, declared with the stdout host module.
 *
 * The handle and its prototype are frozen, null-prototype so it isn't thenable even with a
 * polluted `Object.prototype.then`, and its `constructor` refuses without a key only Gro holds.
 * Read on any other object `is_terminal` throws `TypeError`, and `write` rejects with it, private
 * fields being the only state.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can write to `process.stderr`.
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
import type { ProcessOutput } from './stdout.ts';

/**
 * Stderr.
 */
export const stderr: ProcessOutput = create_process_output('stderr', 2);
