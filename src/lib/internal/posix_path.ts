/**
 * `/`-separated paths as text, read lexically as `node:path`'s POSIX functions read them,
 * for the modules that load as a package on the runtime, where `node:path` doesn't.
 * Nothing here touches the filesystem: a `..` cancels the segment before it whatever that
 * segment names, a symlink included.
 *
 * @module
 */

/**
 * `path` with `.` segments and repeated slashes dropped and each `..` cancelling the segment
 * before it, as `posix.normalize`: an absolute path never climbs above `/`, a relative one keeps
 * the `..` segments it can't cancel, a trailing slash is kept, and an empty result is `.`
 * (`./` with a trailing slash).
 */
export const posix_normalize = (path: string): string => {
	if (path === '') return '.';
	const absolute = path.startsWith('/');
	const trailing = path.endsWith('/');
	const segments: Array<string> = [];
	for (const segment of path.split('/')) {
		if (segment === '' || segment === '.') continue;
		if (segment !== '..') segments.push(segment);
		else if (segments.length > 0 && segments.at(-1) !== '..') segments.pop();
		else if (!absolute) segments.push('..');
	}
	const joined = segments.join('/');
	if (joined === '') return absolute ? '/' : trailing ? './' : '.';
	return (absolute ? '/' : '') + joined + (trailing ? '/' : '');
};

/**
 * The non-empty `parts` joined by `/` and normalized (`posix_normalize`), or `.` when every one
 * is empty, as `posix.join`.
 */
export const posix_join = (...parts: ReadonlyArray<string>): string => {
	const joined = parts.filter((part) => part !== '').join('/');
	return joined === '' ? '.' : posix_normalize(joined);
};

/**
 * `path` against the absolute directory `base`, normalized and without a trailing slash,
 * as `posix.resolve(base, path)`: an absolute `path` stands alone.
 * @throws `TypeError` - when `base` isn't absolute, which `posix.resolve` would read against
 * the working directory
 */
export const posix_resolve = (base: string, path: string): string => {
	if (!base.startsWith('/')) throw new TypeError(`posix_resolve takes an absolute base: ${base}`);
	const resolved = posix_normalize(path.startsWith('/') ? path : base + '/' + path);
	return resolved.length > 1 && resolved.endsWith('/') ? resolved.slice(0, -1) : resolved;
};

/**
 * The directory part of `path`, trailing slashes ignored, as `posix.dirname`:
 * `/` for a path just beneath the root, `.` for a relative path of one segment.
 */
export const posix_dirname = (path: string): string => {
	if (path === '') return '.';
	const absolute = path.startsWith('/');
	let end = -1;
	let in_trailing_slashes = true;
	for (let i = path.length - 1; i >= 1; i--) {
		if (path[i] === '/') {
			if (!in_trailing_slashes) {
				end = i;
				break;
			}
		} else {
			in_trailing_slashes = false;
		}
	}
	if (end === -1) return absolute ? '/' : '.';
	if (absolute && end === 1) return '//';
	return path.slice(0, end);
};

/**
 * The last segment of `path`, trailing slashes ignored, as `posix.basename`,
 * `''` for `/` or an empty path.
 */
export const posix_basename = (path: string): string => {
	let start = 0;
	let end = -1;
	for (let i = path.length - 1; i >= 0; i--) {
		if (path[i] === '/') {
			if (end !== -1) {
				start = i + 1;
				break;
			}
		} else if (end === -1) {
			end = i + 1;
		}
	}
	return end === -1 ? '' : path.slice(start, end);
};

/**
 * The extension of `path`'s last segment, from its last `.`, as `posix.extname`:
 * `''` when the segment has no `.` but a leading one, or is `..`.
 */
export const posix_extname = (path: string): string => {
	const base = posix_basename(path);
	const index = base.lastIndexOf('.');
	return index <= 0 || base === '..' ? '' : base.slice(index);
};

/**
 * The path from the absolute directory `from` to the absolute `to`, as `posix.relative`:
 * `''` when they name the same path, `..` segments up to their common ancestor otherwise.
 * @throws `TypeError` - when either isn't absolute, which `posix.relative` would read against
 * the working directory
 */
export const posix_relative = (from: string, to: string): string => {
	if (!from.startsWith('/') || !to.startsWith('/')) {
		throw new TypeError(`posix_relative takes absolute paths: ${from}, ${to}`);
	}
	const from_parts = to_segments(posix_resolve('/', from));
	const to_parts = to_segments(posix_resolve('/', to));
	let common = 0;
	while (
		common < from_parts.length &&
		common < to_parts.length &&
		from_parts[common] === to_parts[common]
	) {
		common++;
	}
	return [...from_parts.slice(common).map(() => '..'), ...to_parts.slice(common)].join('/');
};

// the segments of a resolved absolute path, none for `/`
const to_segments = (resolved: string): Array<string> =>
	resolved === '/' ? [] : resolved.slice(1).split('/');
