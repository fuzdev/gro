/**
 * The fs handles this copy of Gro made, so another host module can tell a real handle from an
 * object shaped like one without linking `fs.ts`.
 *
 * `fs.ts` registers each handle as it constructs it, with a resolver bound to that handle's own
 * directory. A lookup grants nothing: it takes a handle, and gives back only the confinement
 * that handle already has. Registering is `fs.ts`'s alone by convention: on Node, code that
 * imports this file by its path can register any object, a gap the host module docs list.
 * Importing this module links no host module, as it imports nothing, so `spawn.ts` and
 * `watch.ts` check a handle through it and still don't link `fs`.
 *
 * @module
 */

/**
 * A path resolved beneath a handle by the handle's own rules.
 */
export interface FsHandleResolution {
	/**
	 * The handle's real path joined with the requested path, its `.` and `..` segments
	 * resolved lexically, symlinks kept: where the path was found.
	 */
	readonly found_path: string;
	/**
	 * The path an op runs on, every symlink the walk met followed and confined,
	 * or for a missing entry its parent's real path joined with the rest.
	 * A requested trailing `/` is kept, so the kernel requires a directory.
	 */
	readonly op_path: string;
}

/**
 * Resolves `path` beneath one handle, following a final symlink.
 * @throws `TypeError` - for a path that isn't a well-formed string or holds a NUL byte
 * @throws `FsError` - `outside_root` for an absolute path or one leaving the handle,
 * through its `..` segments or a symlink, and any other kind the walk meets
 */
export type FsHandleResolve = (path: string) => FsHandleResolution;

const handles = new WeakMap<object, FsHandleResolve>();

// uncurried at load, so a later patch of `WeakMap.prototype` can't answer a lookup
const { call } = Function.prototype;
const map_get = call.bind(WeakMap.prototype.get) as unknown as (
	map: WeakMap<object, FsHandleResolve>,
	key: object
) => FsHandleResolve | undefined;
const map_has = call.bind(WeakMap.prototype.has) as unknown as (
	map: WeakMap<object, FsHandleResolve>,
	key: object
) => boolean;
const map_set = call.bind(WeakMap.prototype.set) as unknown as (
	map: WeakMap<object, FsHandleResolve>,
	key: object,
	value: FsHandleResolve
) => unknown;

/**
 * Registers a handle `fs.ts` just constructed, once.
 * @throws `TypeError` - for a handle already registered, so no later registration replaces
 * a real handle's resolver
 */
export const register_fs_handle = (handle: object, resolve: FsHandleResolve): void => {
	if (map_has(handles, handle)) throw new TypeError('an fs handle is registered once');
	map_set(handles, handle, resolve);
};

/**
 * The resolver of a handle this copy of Gro made.
 * @throws `TypeError` - for anything else, a forged object or a handle from another copy of Gro
 */
export const to_fs_handle_resolve = (handle: unknown): FsHandleResolve => {
	const resolve =
		typeof handle === 'object' && handle !== null ? map_get(handles, handle) : undefined;
	if (resolve === undefined) {
		throw new TypeError('expected an fs directory handle from this copy of Gro');
	}
	return resolve;
};
