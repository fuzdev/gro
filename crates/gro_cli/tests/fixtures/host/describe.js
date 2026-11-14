// What a case prints of a value: its shape as data, with no value that differs from run to
// run, so the Node implementation's output is the expectation the builtin's is held to.

const INTRINSIC_PROTOTYPES = [
	['Array.prototype', Array.prototype],
	['Function.prototype', Function.prototype],
	['Object.prototype', Object.prototype]
];

/**
 * The name of `value`'s prototype among the intrinsics a host module's values have,
 * `null` for none, or `other`.
 * @param {object} value
 */
const prototype_name = (value) => {
	const prototype = Object.getPrototypeOf(value);
	if (prototype === null) return null;
	return INTRINSIC_PROTOTYPES.find(([, intrinsic]) => intrinsic === prototype)?.[0] ?? 'other';
};

/**
 * The shape of `value`: its type, and for an object or function its prototype, whether it's
 * frozen, and each own property's descriptor with the type of its value, never the value.
 * @param {unknown} value
 */
export const describe = (value) => {
	if (value === null) return { type: 'null' };
	if (typeof value !== 'object' && typeof value !== 'function') return { type: typeof value };
	/** @type {Record<string, unknown>} */
	const own = {};
	for (const key of Reflect.ownKeys(value)) {
		const descriptor = /** @type {PropertyDescriptor} */ (
			Reflect.getOwnPropertyDescriptor(value, key)
		);
		own[String(key)] =
			'value' in descriptor
				? {
						type: typeof descriptor.value,
						writable: descriptor.writable,
						enumerable: descriptor.enumerable,
						configurable: descriptor.configurable
					}
				: {
						accessor: true,
						enumerable: descriptor.enumerable,
						configurable: descriptor.configurable
					};
	}
	return {
		type: typeof value,
		prototype: prototype_name(value),
		frozen: Object.isFrozen(value),
		own
	};
};

/**
 * The class of what `call` throws, or `null` when it returns.
 * @param {() => unknown} call
 */
export const thrown = (call) => {
	try {
		call();
		return null;
	} catch (error) {
		return /** @type {any} */ (error)?.constructor?.name ?? typeof error;
	}
};
