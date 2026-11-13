/**
 * The null-prototype records the host modules build their values from.
 *
 * Copies `Object`'s functions when it loads, before any host module runs, so a later patch of
 * `Object.assign` or `Object.create` can't change what a record is. Imports nothing, so every
 * host module can share it without linking another.
 *
 * @module
 */

const { assign, create } = Object;

/**
 * A null-prototype record of `fields`, so no name resolves through `Object.prototype`
 * and awaiting one adopts no polluted `Object.prototype.then`.
 */
export const create_record = <T extends object>(fields: T): T => assign(create(null) as T, fields);
