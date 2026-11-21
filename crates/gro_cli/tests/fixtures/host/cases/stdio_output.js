import { describe, thrown } from '../describe.js';

/**
 * What `call`'s promise settled to, or the class of what the call threw before it returned
 * one: `{fulfilled}` with the value, or `{rejected}` or `{threw}` with the class.
 * @param {() => Promise<unknown>} call
 */
const settled = async (call) => {
	let promise;
	try {
		promise = call();
	} catch (error) {
		return { threw: /** @type {any} */ (error)?.constructor?.name ?? typeof error };
	}
	try {
		return { fulfilled: await promise };
	} catch (error) {
		return { rejected: /** @type {any} */ (error)?.constructor?.name ?? typeof error };
	}
};

/**
 * What an output stream's handle is to a program, `ns[name]`, and what its `write` does with
 * every kind of data and receiver, writing nothing, since the case's own output is its stdout
 * and its stderr is empty: the shapes, each write of no bytes resolving `true`, each data that
 * isn't a well-formed string or a live `Uint8Array` refused, and each receiver that isn't the
 * handle refused, `other` the other stream's handle.
 * @param {Record<string, any>} ns
 * @param {string} name
 * @param {any} other
 */
export const output_case = async (ns, name, other) => {
	const handle = ns[name];
	const { write } = handle;
	const detached = new Uint8Array(4);
	detached.buffer.transfer();
	const resizable = new ArrayBuffer(8, { maxByteLength: 8 });
	const out_of_bounds = new Uint8Array(resizable, 0, 4);
	resizable.resize(2);
	class Bytes extends Uint8Array {}
	/** @type {Record<string, unknown>} */
	const refused = {
		lone_surrogate: '\ud800',
		boxed_string: new String(''),
		number: 0,
		null: null,
		undefined,
		array: [],
		array_buffer: new ArrayBuffer(0),
		data_view: new DataView(new ArrayBuffer(0)),
		uint16: new Uint16Array(0),
		proxy: new Proxy(new Uint8Array(0), {}),
		detached
	};
	/** @type {Record<string, unknown>} */
	const refusals = {};
	for (const [kind, data] of Object.entries(refused)) {
		refusals[kind] = await settled(() => handle.write(data));
	}
	return {
		namespace: describe(ns),
		handle: describe(handle),
		prototype: describe(Object.getPrototypeOf(handle)),
		frozen: Object.isFrozen(handle),
		is_terminal: handle.is_terminal,
		write: describe(write),
		write_name: write.name,
		write_length: write.length,
		write_new: thrown(() => new write('')),
		empty: {
			string: await settled(() => handle.write('')),
			bytes: await settled(() => handle.write(new Uint8Array(0))),
			subclass: await settled(() => handle.write(new Bytes(0))),
			view_at_the_end: await settled(() => handle.write(new Uint8Array(new ArrayBuffer(4), 4))),
			unawaited: handle.write('') instanceof Promise
		},
		refusals,
		out_of_bounds: await settled(() => handle.write(out_of_bounds)),
		receivers: {
			plain: await settled(() => write.call({}, '')),
			undefined: await settled(() => write.call(undefined, '')),
			copy: await settled(() => write.call({ ...handle }, '')),
			proxy: await settled(() => write.call(new Proxy(handle, {}), '')),
			other: await settled(() => write.call(other, ''))
		}
	};
};
