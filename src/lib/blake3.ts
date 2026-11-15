/**
 * The blake3 host module: BLAKE3 hashing over bytes, one-shot or incremental, reaching nothing.
 *
 * A digest is 32 bytes, a new `Uint8Array` each call. Bytes are a `Uint8Array`, its subclasses
 * included, and anything else, a proxy of one among them, is a `TypeError`, so nothing a caller
 * passes is converted or iterated. A view is read as it is when the call runs: one of a
 * detached buffer is a `TypeError`, and a fixed-length one a resize left out of its buffer's
 * bounds is empty. A derive-key context is a string, encoded as UTF-8 with each lone
 * surrogate replaced, as `TextEncoder` encodes it.
 *
 * A hasher holds its input's state, not its input, so a stream of any length is hashed in
 * constant memory: `update` it with each chunk, and `finalize` for the digest of what it was
 * given so far, as often as needed. Its state lives as long as it can be reached, with no call
 * to end it, and a hasher is frozen, null-prototype, and recognized by its methods, which throw
 * a `TypeError` called on any other object.
 *
 * This is the Node implementation, which runs on the Node Gro, over `@fuzdev/blake3-wasm`. On
 * `fgro` the module is the runtime's own, over the native `blake3` crate, and grant-free, so the
 * project and every package may import it. Its gap against the runtime's builtin, which it
 * documents rather than the API weakening to match: a hasher's state is in the WASM module's
 * memory, released when the hasher is collected, by the package's `FinalizationRegistry`.
 *
 * @module
 */

import { Blake3Hasher as WasmHasher, hash as wasm_hash } from '@fuzdev/blake3-wasm';
import { isUint8Array } from 'node:util/types';

import { freeze_methods } from './internal/records.ts';

const { freeze, setPrototypeOf } = Object;

const NOT_BYTES = 'blake3 hashes a Uint8Array';
const NOT_A_HASHER = "a hasher's methods are called on a hasher, not another object";

/**
 * A hasher: BLAKE3's state over the input it has been given.
 */
export interface Blake3Hasher {
	/**
	 * Adds `data` to the input.
	 * @throws `TypeError` - for `data` that isn't a `Uint8Array`
	 */
	update(data: Uint8Array): void;
	/**
	 * The digest of the input so far, 32 bytes, leaving the hasher as it was.
	 */
	finalize(): Uint8Array;
}

const DETACHED = "blake3 can't read a view of a detached buffer";

// what a view with no bytes that isn't detached is passed as: the package refuses a view a
// resize left out of its buffer's bounds, which the runtime's module reads as empty
const EMPTY = new Uint8Array(0);

/**
 * `value` as bytes, a `Uint8Array`, or a `TypeError` of `message`; a view of a detached buffer is
 * a `TypeError` too, as the package throws for one. Another view with no bytes, one a resize left
 * out of its buffer's bounds among them, is an empty array.
 */
const check_bytes = (value: unknown, message: string): Uint8Array => {
	if (!isUint8Array(value)) throw new TypeError(message);
	if (value.byteLength !== 0) return value;
	if ((value.buffer as ArrayBuffer).detached) throw new TypeError(DETACHED);
	return EMPTY;
};

// the key only this module holds, so a hasher's `constructor` can't make another hasher
const HASHER_KEY = Symbol('blake3 hasher');

class NodeHasher implements Blake3Hasher {
	readonly #inner: WasmHasher;

	constructor(key: symbol, inner: WasmHasher) {
		if (key !== HASHER_KEY) {
			throw new TypeError('blake3 hashers come from its factories, not a constructor');
		}
		this.#inner = inner;
	}

	/**
	 * The state of `hasher`, or a `TypeError` when it's no hasher: a proxy of one is none.
	 */
	static #inner_of(hasher: unknown): WasmHasher {
		if (typeof hasher !== 'object' || hasher === null || !(#inner in hasher)) {
			throw new TypeError(NOT_A_HASHER);
		}
		return hasher.#inner;
	}

	update(data: Uint8Array): void {
		const inner = NodeHasher.#inner_of(this);
		inner.update(check_bytes(data, NOT_BYTES));
	}

	finalize(): Uint8Array {
		return NodeHasher.#inner_of(this).finalize();
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
setPrototypeOf(NodeHasher.prototype, null);
freeze_methods(NodeHasher.prototype);
freeze(NodeHasher);

/**
 * A hasher of `inner`, frozen.
 */
const to_hasher = (inner: WasmHasher): Blake3Hasher => freeze(new NodeHasher(HASHER_KEY, inner));

/**
 * The BLAKE3 digest of `data`, 32 bytes.
 * @throws `TypeError` - for `data` that isn't a `Uint8Array`
 */
export const hash = (data: Uint8Array): Uint8Array => wasm_hash(check_bytes(data, NOT_BYTES));

/**
 * A new hasher, for BLAKE3's plain hash, the one `hash` computes.
 */
export const create_hasher = (): Blake3Hasher => to_hasher(new WasmHasher());

/**
 * A new hasher for BLAKE3's keyed hash, a MAC, of `key`.
 * @throws `TypeError` - for a key that isn't a `Uint8Array`
 * @throws `RangeError` - for a key that isn't 32 bytes
 */
export const create_keyed_hasher = (key: Uint8Array): Blake3Hasher => {
	const checked = check_bytes(key, "a keyed hasher's key is a Uint8Array");
	if (checked.length !== 32) throw new RangeError("a keyed hasher's key is exactly 32 bytes");
	return to_hasher(WasmHasher.new_keyed(checked));
};

/**
 * A new hasher for BLAKE3's key derivation in `context`, a string that names the application,
 * whose digest of the key material it's given is the derived key.
 * @throws `TypeError` - for a context that isn't a string
 */
export const create_derive_key_hasher = (context: string): Blake3Hasher => {
	if (typeof context !== 'string') {
		throw new TypeError("a derive_key hasher's context is a string");
	}
	return to_hasher(WasmHasher.new_derive_key(context));
};

freeze(hash);
freeze(create_hasher);
freeze(create_keyed_hasher);
freeze(create_derive_key_hasher);
