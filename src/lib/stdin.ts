/**
 * The stdin host module: the running process's standard input, as an import rather than a global.
 *
 * Handles and their prototypes are frozen, null-prototype so none of them is thenable even with a
 * polluted `Object.prototype.then`, and constructing one takes a key only this module holds.
 * A method called on any other object throws `TypeError`, private fields being the only state.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can read `process.stdin`.
 * - Intrinsics stay pollutable. The stream classes and `EventEmitter.prototype` (`emit`) stay,
 *   so code that patches them runs inside these ops holding the raw stream.
 * - A chunk read from stdin passes through Node's iterator, so a polluted `then` can replace it.
 *
 * @module
 */

import * as node_tty from 'node:tty';
import * as node_util from 'node:util';

import { to_chunk_result } from './internal/records.ts';
import { StdioError } from './stdio_error.ts';

const host_process = globalThis.process;
const { isatty } = node_tty;
const { isUint8Array } = node_util.types;
const { freeze, setPrototypeOf } = Object;
const Uint8ArrayClass = Uint8Array;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('stdin handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('stdin handles come from `stdin`, not a constructor');
	}
};

/**
 * The process's stdin.
 */
export interface ProcessInput extends AsyncIterable<Uint8Array> {
	/**
	 * Whether stdin is a terminal.
	 */
	readonly is_terminal: boolean;
}

class NodeProcessInput implements ProcessInput {
	readonly #fd = 0;
	#claimed = false;

	constructor(key: symbol) {
		check_handle_key(key);
	}

	get is_terminal(): boolean {
		return isatty(this.#fd);
	}

	[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
		if (this.#claimed) throw new TypeError('stdin has one reader, once');
		this.#claimed = true;
		return freeze(new NodeProcessInputIterator(HANDLE_KEY, host_process.stdin));
	}
}

class NodeProcessInputIterator implements AsyncIterator<Uint8Array> {
	readonly #stream: NodeJS.ReadStream;
	readonly #inner: AsyncIterator<Uint8Array>;

	constructor(key: symbol, stream: NodeJS.ReadStream) {
		check_handle_key(key);
		this.#stream = stream;
		// not destroying, as closing fd 0 would take stdin from any child started after
		this.#inner = stream.iterator({ destroyOnReturn: false });
	}

	async next(): Promise<IteratorResult<Uint8Array>> {
		// read before the `try`, so a borrowed receiver is a `TypeError`, not a stdin failure
		const inner = this.#inner;
		let result: IteratorResult<Uint8Array>;
		try {
			result = await inner.next();
		} catch (error) {
			throw new StdioError('io_error', 'stdin', `io_error: stdin: ${(error as Error).message}`, {
				cause: error
			});
		}
		if (result.done) return to_chunk_result(null);
		const { value } = result;
		// not bytes if something set an encoding on the stream, or forged it to something else through `then`
		if (!isUint8Array(value)) {
			await this.#stop();
			throw new StdioError('io_error', 'stdin', "io_error: stdin: a chunk isn't bytes");
		}
		// a copy, so a reader never holds a `Buffer` or a view of memory Node reuses
		return to_chunk_result(new Uint8ArrayClass(value));
	}

	// reached by leaving a `for await` early
	async return(): Promise<IteratorResult<Uint8Array>> {
		await this.#stop();
		return to_chunk_result(null);
	}

	/**
	 * Stops the read, so a child inheriting stdin gets what's left and the host can exit.
	 */
	async #stop(): Promise<void> {
		const stream = this.#stream;
		await this.#inner.return?.();
		stream.pause();
		// Node's stdin stops reading on `'pause'`, which `pause` doesn't emit here since the iterator's
		// `'readable'` listener left the stream not flowing; emitted now, before the loop can poll again
		stream.emit('pause');
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
for (const handle_class of [NodeProcessInput, NodeProcessInputIterator]) {
	setPrototypeOf(handle_class.prototype, null);
	freeze(handle_class.prototype);
	freeze(handle_class);
}

/**
 * Stdin. Iterating is once only and reads bytes as plain `Uint8Array` copies, a second
 * iteration throwing `TypeError`. Leaving a loop early stops reading.
 */
export const stdin: ProcessInput = freeze(new NodeProcessInput(HANDLE_KEY));
