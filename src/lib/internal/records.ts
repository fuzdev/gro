/**
 * The null-prototype records the host modules build their values from, and the own-property
 * read of the options they're given.
 *
 * Copies `Object`'s and `Reflect`'s functions when it loads, before the host module importing it
 * runs, so a later patch of `Object.assign`, `Object.create`, `Object.hasOwn`, `Object.freeze`, or
 * `Reflect.ownKeys` can't change what a record is, which options are read, or what's frozen. Imports nothing, so every host module can share it without
 * linking another.
 *
 * @module
 */

const { assign, create, freeze, getOwnPropertyDescriptor, hasOwn } = Object;
const { ownKeys } = Reflect;

/**
 * A null-prototype record of `fields`, so no name resolves through `Object.prototype`
 * and awaiting one adopts no polluted `Object.prototype.then`.
 */
export const create_record = <T extends object>(fields: T): T => assign(create(null) as T, fields);

/**
 * A byte stream's iterator result, a null-prototype record, so a polluted `then` isn't
 * consulted when a reader's `next()` resolves with one.
 * @param chunk - the next chunk, or `null` when the stream is done
 */
export const to_chunk_result = (chunk: Uint8Array | null): IteratorResult<Uint8Array> =>
	chunk === null
		? create_record<IteratorResult<Uint8Array>>({ done: true, value: undefined })
		: create_record<IteratorResult<Uint8Array>>({ done: false, value: chunk });

/**
 * An option's value, read as an own property so a polluted `Object.prototype` can't set one.
 */
export const read_own = (options: object | null | undefined, key: string): unknown =>
	options !== null && options !== undefined && hasOwn(options, key)
		? (options as Record<string, unknown>)[key]
		: undefined;

/**
 * Freezes each function `object` holds as an own property, a method's or an accessor's,
 * with a generator method's own `prototype`, the prototype of the iterators it makes,
 * then `object`: a handle's prototype or a root record, whose functions every holder of a
 * handle shares, so none can leave a value on one, or on its iterators, for another to find,
 * and writing a property on one is a `TypeError` in strict mode.
 * Once at load, for each prototype and root record of the host modules.
 * @returns `object`, frozen
 */
export const freeze_methods = <T extends object>(object: T): T => {
	const keys = ownKeys(object);
	for (let i = 0; i < keys.length; i++) {
		const { value, get, set } = getOwnPropertyDescriptor(object, keys[i]!)!;
		if (typeof value === 'function') {
			// a generator method's own `prototype` is the prototype of every iterator it makes,
			// which every holder's calls share; a class's is the object being frozen here
			const prototype = getOwnPropertyDescriptor(value, 'prototype');
			if (prototype !== undefined && typeof prototype.value === 'object') freeze(prototype.value);
			freeze(value);
		}
		if (get !== undefined) freeze(get);
		if (set !== undefined) freeze(set);
	}
	return freeze(object);
};
