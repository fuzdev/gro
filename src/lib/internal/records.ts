/**
 * The null-prototype records the host modules build their values from, and the own-property
 * read of the options they're given.
 *
 * Copies `Object`'s functions when it loads, before the host module importing it runs, so a
 * later patch of `Object.assign`, `Object.create`, or `Object.hasOwn` can't change what a record
 * is or which options are read. Imports nothing, so every host module can share it without
 * linking another.
 *
 * @module
 */

const { assign, create, hasOwn } = Object;

/**
 * A null-prototype record of `fields`, so no name resolves through `Object.prototype`
 * and awaiting one adopts no polluted `Object.prototype.then`.
 */
export const create_record = <T extends object>(fields: T): T => assign(create(null) as T, fields);

/**
 * An option's value, read as an own property so a polluted `Object.prototype` can't set one.
 */
export const read_own = (options: object | null | undefined, key: string): unknown =>
	options !== null && options !== undefined && hasOwn(options, key)
		? (options as Record<string, unknown>)[key]
		: undefined;
