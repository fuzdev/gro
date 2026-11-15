import { describe, thrown } from '../describe.js';

/** What `call` throws, its class and message, or `null` when it returns. */
const refusal = (/** @type {() => unknown} */ call) => {
	try {
		call();
		return null;
	} catch (error) {
		const { constructor, message } = /** @type {any} */ (error);
		return [constructor?.name ?? typeof error, message];
	}
};

/** `bytes` in hex. */
const hex = (/** @type {Uint8Array} */ bytes) => {
	let text = '';
	for (const byte of bytes) text += byte.toString(16).padStart(2, '0');
	return text;
};

/** The official test vectors' input of `length` bytes: byte `i` is `i % 251`. */
const pattern = (/** @type {number} */ length) => {
	const bytes = new Uint8Array(length);
	for (let i = 0; i < length; i++) bytes[i] = i % 251;
	return bytes;
};

/** The official test vectors' key and context. */
const KEY = new TextEncoder().encode('whats the Elvish word for friend');
const CONTEXT = 'BLAKE3 2019-12-27 16:29:52 test vectors context';

/**
 * The first 32 bytes of the official test vectors' extended outputs at a few lengths, by
 * length: the plain hash, the keyed hash, and the derived key.
 */
const OFFICIAL = {
	0: [
		'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262',
		'92b2b75604ed3c761f9d6f62392c8a9227ad0ea3f09573e783f1498a4ed60d26',
		'2cc39783c223154fea8dfb7c1b1660f2ac2dcbd1c1de8277b0b0dd39b7e50d7d'
	],
	1024: [
		'42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7',
		'75c46f6f3d9eb4f55ecaaee480db732e6c2105546f1e675003687c31719c7ba4',
		'7356cd7720d5b66b6d0697eb3177d9f8d73a4a5c5e968896eb6a689684302706'
	],
	1025: [
		'd00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444',
		'357dc55de0c7e382c900fd6e320acc04146be01db6a8ce7210b7189bd664ea69',
		'effaa245f065fbf82ac186839a249707c3bddf6d3fdda22d1b95a3c970379bcb'
	],
	8193: [
		'bab6c09cb8ce8cf459261398d2e7aef35700bf488116ceb94a36d0f5f1b7bc3b',
		'954a2a75420c8d6547e3ba5b98d963e6fa6491addc8c023189cc519821b4a1f5',
		'af1e0346e389b17c23200270a64aa4e1ead98c61695d917de7d5b00491c9b0f1'
	]
};

/** The official test vectors' input lengths, around the block and chunk boundaries. */
const LENGTHS = [
	0, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 2049, 3072, 3073, 4096, 4097, 5120, 5121, 6144, 6145,
	7168, 7169, 8192, 8193, 16384, 31744, 102400
];

/**
 * The digest of `data` by `hasher`, given it in chunks of `size`, then the whole digest again.
 * @param {{update: (data: Uint8Array) => void, finalize: () => Uint8Array}} hasher
 * @param {Uint8Array} data
 * @param {number} size
 */
const in_chunks = (hasher, data, size) => {
	for (let at = 0; at < data.length; at += size) hasher.update(data.subarray(at, at + size));
	return hex(hasher.finalize());
};

/**
 * The scenarios of the blake3 module, by name.
 * @type {Record<string, (ns: typeof import('../../../../../../src/lib/blake3.ts')) => unknown>}
 */
const SCENARIOS = {
	// the namespace, each function, a hasher, its prototype, and each method
	shape: (ns) => {
		const { hash, create_hasher, create_keyed_hasher, create_derive_key_hasher } = ns;
		const hasher = create_hasher();
		const prototype = Object.getPrototypeOf(hasher);
		const digest = hash(new Uint8Array(1));
		return {
			namespace: describe(ns),
			functions: [hash, create_hasher, create_keyed_hasher, create_derive_key_hasher].map((fn) => ({
				shape: describe(fn),
				name: fn.name,
				length: fn.length
			})),
			hasher: describe(hasher),
			prototype: describe(prototype),
			prototype_prototype: Object.getPrototypeOf(prototype),
			methods: ['update', 'finalize'].map((name) => {
				const method = prototype[name];
				return { name, shape: describe(method), length: method.length, name_of: method.name };
			}),
			one_prototype:
				Object.getPrototypeOf(create_keyed_hasher(KEY)) === prototype &&
				Object.getPrototypeOf(create_derive_key_hasher(CONTEXT)) === prototype,
			another_each_call: create_hasher() !== hasher,
			digest: {
				uint8array: digest instanceof Uint8Array,
				length: digest.length,
				byte_offset: digest.byteOffset,
				own_buffer: digest.buffer.byteLength === 32,
				another_each_call: hash(new Uint8Array(1)) !== digest,
				another_buffer_each_call: hash(new Uint8Array(1)).buffer !== digest.buffer,
				another_each_finalize: hasher.finalize() !== hasher.finalize(),
				writable: (() => {
					digest[0] = 0;
					return digest[0] === 0;
				})()
			},
			json: JSON.stringify(hasher),
			thenable: 'then' in hasher,
			own_keys: Reflect.ownKeys(hasher),
			new_hash: thrown(() => new /** @type {any} */ (hash)(new Uint8Array())),
			new_create_hasher: thrown(() => new /** @type {any} */ (create_hasher)()),
			new_update: thrown(() => new /** @type {any} */ (hasher.update)(new Uint8Array())),
			property_write: thrown(() => {
				/** @type {any} */ (hasher).extra = 1;
			}),
			method_write: thrown(() => {
				prototype.update = () => {};
			}),
			// a value one holder of a hasher could leave on a method for another
			shell_write: thrown(() => {
				prototype.update.stash = 'x';
			}),
			function_write: thrown(() => {
				/** @type {any} */ (hash).stash = 'x';
			}),
			namespace_write: thrown(() => {
				/** @type {any} */ (ns).hash = null;
			})
		};
	},
	// the official test vectors' inputs, each mode one-shot where it is and by a hasher
	vectors: (ns) => {
		const { hash, create_hasher, create_keyed_hasher, create_derive_key_hasher } = ns;
		const digests = LENGTHS.map((length) => {
			const input = pattern(length);
			const plain = hex(hash(input));
			const keyed = create_keyed_hasher(KEY);
			keyed.update(input);
			const derived = create_derive_key_hasher(CONTEXT);
			derived.update(input);
			const by_hasher = create_hasher();
			by_hasher.update(input);
			return {
				length,
				hash: plain,
				keyed_hash: hex(keyed.finalize()),
				derive_key: hex(derived.finalize()),
				hasher_is_hash: hex(by_hasher.finalize()) === plain
			};
		});
		return {
			digests,
			official: Object.entries(OFFICIAL).map(([length, expected]) => {
				const found = digests.find((digest) => digest.length === Number(length));
				return [
					Number(length),
					found?.hash === expected[0],
					found?.keyed_hash === expected[1],
					found?.derive_key === expected[2]
				];
			})
		};
	},
	// a large input, one-shot and by a hasher given it in chunks split at odd boundaries
	incremental: (ns) => {
		const { hash, create_hasher, create_keyed_hasher, create_derive_key_hasher } = ns;
		const input = pattern(2 ** 20 + 1);
		const whole = hex(hash(input));
		const sizes = [1, 63, 64, 65, 1023, 1025, 4097, 65537, 2 ** 20 + 1];
		const keyed_whole = create_keyed_hasher(KEY);
		keyed_whole.update(input);
		const derived_whole = create_derive_key_hasher(CONTEXT);
		derived_whole.update(input);
		const hasher = create_hasher();
		hasher.update(input.subarray(0, 1000));
		const prefix = hex(hasher.finalize());
		const prefix_again = hex(hasher.finalize());
		hasher.update(input.subarray(1000));
		const empty_updates = create_hasher();
		empty_updates.update(new Uint8Array());
		empty_updates.update(input);
		empty_updates.update(new Uint8Array());
		return {
			whole,
			chunks: sizes.map((size) => [size, in_chunks(create_hasher(), input, size) === whole]),
			keyed: sizes.map((size) => [
				size,
				in_chunks(create_keyed_hasher(KEY), input, size) === hex(keyed_whole.finalize())
			]),
			derived: sizes.map((size) => [
				size,
				in_chunks(create_derive_key_hasher(CONTEXT), input, size) === hex(derived_whole.finalize())
			]),
			prefix: prefix === hex(hash(input.subarray(0, 1000))),
			finalize_leaves_it: prefix === prefix_again,
			continued: hex(hasher.finalize()) === whole,
			empty_updates: hex(empty_updates.finalize()) === whole,
			fresh: hex(create_hasher().finalize()) === hex(hash(new Uint8Array()))
		};
	},
	// what counts as bytes, a key, and a context, read as it is when the call runs
	inputs: (ns) => {
		const { hash, create_hasher, create_keyed_hasher, create_derive_key_hasher } = ns;
		const bytes = pattern(100);
		class SubUint8Array extends Uint8Array {}
		const detached = new Uint8Array(new ArrayBuffer(8));
		detached.buffer.transfer();
		const resizable = new ArrayBuffer(8, { maxByteLength: 64 });
		const tracking = new Uint8Array(resizable);
		tracking.set([1, 2, 3, 4, 5, 6, 7, 8]);
		const before_resize = hex(hash(tracking));
		resizable.resize(16);
		const after_resize = hex(hash(tracking));
		const shared = new Uint8Array(new SharedArrayBuffer(100));
		shared.set(bytes);
		const hasher = create_hasher();
		/** @type {Array<[string, unknown]>} */
		const not_bytes = [
			['undefined', undefined],
			['null', null],
			['a number', 1],
			['a string', 'abc'],
			['an array', [1, 2, 3]],
			['an array buffer', bytes.buffer],
			['a data view', new DataView(bytes.buffer)],
			['a Uint16Array', new Uint16Array(4)],
			['an Int8Array', new Int8Array(4)],
			['a Uint8ClampedArray', new Uint8ClampedArray(4)],
			['a proxy of a Uint8Array', new Proxy(bytes, {})],
			['an object', { length: 1, 0: 1 }]
		];
		return {
			subclass: hex(hash(SubUint8Array.from(bytes))) === hex(hash(bytes)),
			subarray: hex(hash(bytes.subarray(10, 20))) === hex(hash(pattern(100).slice(10, 20))),
			// a view of a detached buffer is refused, as the package refuses one on Node, and the
			// hasher is left as it was
			detached: refusal(() => hash(detached)),
			detached_update: (() => {
				const detached_hasher = create_hasher();
				return [
					refusal(() => detached_hasher.update(detached)),
					hex(detached_hasher.finalize()) === hex(hash(new Uint8Array()))
				];
			})(),
			// a fixed-length view a resize left out of bounds reads as empty, where the package
			// throws: V8's API says nothing of a view's bounds
			out_of_bounds: (() => {
				const shrinking = new ArrayBuffer(8, { maxByteLength: 8 });
				const fixed = new Uint8Array(shrinking, 0, 8);
				shrinking.resize(4);
				return hex(hash(fixed)) === hex(hash(new Uint8Array()));
			})(),
			before_resize: before_resize === hex(hash(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]))),
			after_resize:
				after_resize ===
				hex(hash(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0]))),
			shared: hex(hash(shared)) === hex(hash(bytes)),
			hash: not_bytes.map(([what, value]) => [
				what,
				refusal(() => hash(/** @type {any} */ (value)))
			]),
			update: not_bytes.map(([what, value]) => [
				what,
				refusal(() => hasher.update(/** @type {any} */ (value)))
			]),
			// a refused update leaves the hasher as it was
			after_refusals: hex(hasher.finalize()) === hex(hash(new Uint8Array())),
			keys: [
				['31 bytes', new Uint8Array(31)],
				['33 bytes', new Uint8Array(33)],
				['empty', new Uint8Array()],
				['a detached one', detached],
				['an array of 32', Array.from({ length: 32 }, () => 0)],
				['a string of 32', 'x'.repeat(32)],
				['a proxy of 32 bytes', new Proxy(new Uint8Array(32), {})],
				['undefined', undefined]
			].map(([what, key]) => [what, refusal(() => create_keyed_hasher(/** @type {any} */ (key)))]),
			key_of_a_subarray:
				hex(
					(() => {
						const keyed = create_keyed_hasher(new Uint8Array(40).subarray(8));
						return keyed.finalize();
					})()
				) ===
				hex(
					(() => {
						const keyed = create_keyed_hasher(new Uint8Array(32));
						return keyed.finalize();
					})()
				),
			contexts: [
				['undefined', undefined],
				['a number', 1],
				['a boxed string', new String('x')],
				['a symbol', Symbol('x')],
				['bytes', bytes]
			].map(([what, context]) => [
				what,
				refusal(() => create_derive_key_hasher(/** @type {any} */ (context)))
			]),
			// a lone surrogate is encoded as U+FFFD, as `TextEncoder` encodes it
			lone_surrogate_context:
				hex(create_derive_key_hasher('a\uD800b').finalize()) ===
				hex(create_derive_key_hasher('a�b').finalize()),
			empty_context: hex(create_derive_key_hasher('').finalize()),
			unicode_context: hex(create_derive_key_hasher('café \u{1F600}').finalize())
		};
	},
	// a method called on what isn't a hasher, which is refused before its argument is read
	receiver: (ns) => {
		const { create_hasher } = ns;
		const hasher = create_hasher();
		const prototype = Object.getPrototypeOf(hasher);
		/** @type {Array<[string, unknown]>} */
		const receivers = [
			['undefined', undefined],
			['an object', {}],
			['a look-alike', Object.create(prototype)],
			['a proxy of a hasher', new Proxy(hasher, {})],
			['a copy', { ...hasher }],
			['a function', () => {}]
		];
		return {
			update: receivers.map(([what, receiver]) => [
				what,
				refusal(() => prototype.update.call(receiver, new Uint8Array()))
			]),
			update_with_no_bytes: receivers.map(([what, receiver]) => [
				what,
				refusal(() => prototype.update.call(receiver, 'not bytes'))
			]),
			finalize: receivers.map(([what, receiver]) => [
				what,
				refusal(() => prototype.finalize.call(receiver))
			]),
			// a hasher's methods borrowed by another hasher act on that one
			borrowed: (() => {
				const other = create_hasher();
				hasher.update.call(other, new Uint8Array([1]));
				return hex(other.finalize()) !== hex(hasher.finalize());
			})()
		};
	}
};

/**
 * @param {typeof import('../../../../../../src/lib/blake3.ts')} ns
 * @param {Array<string>} rest - the scenario's name
 */
export default (ns, [scenario]) => {
	const run = SCENARIOS[/** @type {string} */ (scenario)];
	if (!run) throw new Error(`no blake3 scenario named ${scenario}`);
	return run(ns);
};
