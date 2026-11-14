/**
 * The Node implementation of an output stream handle, shared by the stdout and stderr host
 * modules, each making its own handle once when it loads.
 *
 * Node-only. Handles and their prototype are frozen, null-prototype so none is thenable even with
 * a polluted `Object.prototype.then`, and constructing one takes a key only this module holds.
 * A method called on any other object throws `TypeError`, private fields being the only state.
 *
 * @module
 */

import * as node_tty from 'node:tty';
import * as node_util from 'node:util';

import { StdioError } from '../stdio_error.ts';
import type { ProcessOutput } from '../stdout.ts';

const host_process = globalThis.process;
const { isatty } = node_tty;
const { isUint8Array } = node_util.types;
const { freeze, setPrototypeOf } = Object;
const Uint8ArrayClass = Uint8Array;
const set_immediate = setImmediate;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('output handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('output handles come from `stdout` and `stderr`, not a constructor');
	}
};

const to_data_chunk = (data: unknown): string | Uint8Array => {
	if (typeof data === 'string') {
		if (!data.isWellFormed()) throw new TypeError("stdio data isn't well-formed Unicode");
		return data;
	}
	if (!isUint8Array(data)) throw new TypeError('stdio data is a string or a `Uint8Array`');
	// a copy, as Node holds what it can't write yet and a caller may reuse the buffer
	return new Uint8ArrayClass(data);
};

const ignore_error = (): void => {
	// each write's callback reports its own failure, so its `'error'` event has nothing to add
};

/**
 * Whether a write failed because the reader went away, the case a `false` result is for.
 * Node leaves `process.stdout` undestroyed after `EPIPE`, every later write erroring the same way.
 */
const is_reader_gone = (error: Error): boolean => {
	const { code } = error as { code?: unknown };
	return (
		code === 'EPIPE' ||
		code === 'ECONNRESET' ||
		code === 'ERR_STREAM_DESTROYED' ||
		code === 'ERR_STREAM_WRITE_AFTER_END'
	);
};

class NodeProcessOutput implements ProcessOutput {
	readonly #target: 'stdout' | 'stderr';
	readonly #fd: number;
	#stream: NodeJS.WriteStream | null = null;

	constructor(key: symbol, target: 'stdout' | 'stderr', fd: number) {
		check_handle_key(key);
		this.#target = target;
		this.#fd = fd;
	}

	get is_terminal(): boolean {
		return isatty(this.#fd);
	}

	/**
	 * The Node stream, made on first use since reading the global creates it.
	 */
	#writable(): NodeJS.WriteStream {
		return (this.#stream ??= host_process[this.#target]);
	}

	write(data: string | Uint8Array): Promise<boolean> {
		const target = this.#target;
		return new Promise((resolve, reject) => {
			const chunk = to_data_chunk(data);
			const stream = this.#writable();
			stream.write(chunk, (error) => {
				if (error === null || error === undefined) {
					resolve(true);
					return;
				}
				// the failure is also emitted as `'error'`, which throws with no listener; a one-shot
				// listener only for this one, as Node's `Console` does, so another writer's still throws
				if (stream.listenerCount('error') === 0) stream.once('error', ignore_error);
				if (is_reader_gone(error)) {
					// on a later turn, so a write loop past a gone reader can't starve signal handling
					set_immediate(resolve, false);
				} else {
					reject(
						new StdioError('io_error', target, `io_error: ${target}: ${error.message}`, {
							cause: error
						})
					);
				}
			});
		});
	}
}

setPrototypeOf(NodeProcessOutput.prototype, null);
freeze(NodeProcessOutput.prototype);
freeze(NodeProcessOutput);

/**
 * The handle of the output stream `target` on descriptor `fd`, frozen.
 * The stdout and stderr host modules each call it once, when they load.
 */
export const create_process_output = (target: 'stdout' | 'stderr', fd: 1 | 2): ProcessOutput =>
	freeze(new NodeProcessOutput(HANDLE_KEY, target, fd));
