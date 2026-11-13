/**
 * The fs host module: filesystem access through directory handles.
 *
 * Code reaches the filesystem only through an `FsDir` (or the read-only `FsReadDir`),
 * a handle on one directory that resolves relative paths beneath it, the shape of
 * cap-std and WASI preopens. The import is the capability, and handing a narrower
 * handle (`dir.dir('sub')`, `dir.readonly()`) down the call graph is how authority narrows.
 * `fs.project` is the project directory, `fs.open_dir` opens any other directory,
 * and `fs.temp_dir` makes a fresh one that removes itself on `await using`.
 * Only `fs` and other handles make handles, and handles and their prototypes are frozen,
 * so a handle's `constructor` makes nothing and no holder can patch another's ops.
 * They and `fs` are null-prototype, so awaiting one adopts no polluted `Object.prototype.then`.
 * Each op reads its handle's state from private fields, never `path`, so an op called
 * on any other object throws `TypeError`.
 *
 * Paths beneath a handle are relative and `/`-separated, and resolve as `openat2` with
 * `RESOLVE_BENEATH` does. An absolute path, or one whose `..` segments climb above the
 * handle, fails with `outside_root`. So does an absolute symlink, and a relative one
 * whose target climbs above the handle, even when it would land back inside
 * (`../root/real`). The path's own `..` segments resolve lexically, before any symlink,
 * so `link/..` is the handle's own directory whatever `link` points at; the runtime's
 * native op resolves them the same way before calling the kernel. A trailing `/`
 * means a directory, as in the kernel: `stat('a.txt/')` and `read('a.txt/')` fail with
 * `not_a_directory`, `write('new.txt/')` fails, and `remove` and `rename` refuse a
 * symlink named with a trailing `/` (`not_a_directory`), never acting on its target.
 * `remove` and `rename` refuse the handle's own directory with `io_error`.
 * Symlink targets, real paths, and listed names are read as bytes, and one that isn't UTF-8 fails
 * with `io_error`, since no string names what the kernel would reach.
 * A symlink's target is data: `symlink` stores any target verbatim and `read_link` returns it,
 * and following a link is what's confined, by whichever handle reaches it.
 *
 * Errors are typed: each op throws an `FsError` (`fs_error.ts`) whose `kind` names the
 * failure, and `is_fs_error` recognizes one from any copy of Gro. Each op's one expected failure
 * returns a value instead of throwing: `stat` returns `null` on `not_found`,
 * `exists` returns `false`, `read_if_exists` and `read_text_if_exists` return `null`,
 * `remove({missing_ok})` and `mkdir({exists_ok})` return normally,
 * and `write({create_new})` and `symlink` return `'exists'`. Every other kind still throws,
 * so a permission or I/O error is never mistaken for a missing file.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - Confinement is a check, then an open: the path is walked beneath the handle's
 *   directory one component at a time, reading each symlink, then the op runs on the
 *   path the walk found, `read_link`, `set_times`, `set_mode`, and `symlink` included.
 *   A symlink swapped in between escapes, since Node has no `openat2`.
 *   The twin is a review signal, not a boundary.
 * - A handle is bound to its directory's path, not its inode: a handle whose directory
 *   is moved or replaced follows the path, refused only when the path's real location
 *   changed (`outside_root`) or is gone (`not_found`). The runtime's handle holds a
 *   descriptor and keeps the directory it opened.
 * - Intrinsics stay pollutable: code that patches `String.prototype`, `Array.prototype`,
 *   or `Promise` after this module loads runs inside its ops. The walk refuses a segment
 *   holding a `/`, so a forged segment can't climb out, and the Node functions it calls
 *   are copied at load, so `syncBuiltinESMExports` can't rebind them.
 * - A handle opened on `/proc` follows its magic links, which the kernel resolves
 *   without a path the walk can check.
 * - `fs.open_dir` takes any absolute path, checked against no grant.
 * - Any module can import `node:fs`, so a narrowed handle bounds only code that
 *   uses the handles.
 * - `fs.project` is the working directory when this module loads, the same source
 *   as Gro's `paths.root`; `getcwd` returns a path with symlinks resolved. Importing
 *   the module throws when the working directory has been removed, or when its path
 *   isn't UTF-8, which Node would report lossily as a different path.
 *
 * @module
 */

import * as node_fs_promises from 'node:fs/promises';
import * as node_fs from 'node:fs';
import type { Dir, Dirent, Stats } from 'node:fs';
import * as node_os from 'node:os';
import * as node_path from 'node:path';

import { FsError, is_own_fs_error, type FsErrorKind } from './fs_error.ts';
import { to_checked_cwd } from './internal/cwd.ts';
import { register_fs_handle, type FsHandleResolution } from './internal/fs_handles.ts';
import { create_record } from './internal/records.ts';

// copied at load rather than imported by name, since a named import of a builtin is a live
// binding that `syncBuiltinESMExports` rebinds to whatever a caller assigned to the export
const {
	chmod,
	copyFile,
	cp,
	lstat,
	lutimes,
	mkdir,
	mkdtemp,
	opendir,
	readFile,
	readlink,
	realpath,
	rename,
	rm,
	rmdir,
	stat,
	symlink,
	unlink,
	utimes,
	writeFile
} = node_fs_promises;
const { lstatSync, readFileSync, readlinkSync, statSync } = node_fs;
// libuv's `realpath`, a syscall per component, where `realpathSync` runs Node's JS walk
const realpath_sync_native = node_fs.realpathSync.native;
const { tmpdir } = node_os;
const { versions } = globalThis.process;
const { dirname, isAbsolute } = node_path;

// TODO: `FsFile` and `dir.open(path)`, a file handle for byte-level reads and writes
// into caller buffers, the native-first byte path, once a caller needs one

/**
 * The kind of a filesystem entry. `stat` with `follow` (the default) never
 * returns `'symlink'`; `stat` without it and `list` do.
 */
export type FileKind = 'file' | 'directory' | 'symlink' | 'other';

/**
 * What `stat` reports about an entry.
 */
export interface FileInfo {
	kind: FileKind;
	/**
	 * Size in bytes.
	 */
	size: number;
	/**
	 * Last modification time in milliseconds since the epoch.
	 */
	mtime_ms: number;
	/**
	 * The permission bits, like `0o644`, without the file type bits.
	 */
	mode: number;
}

/**
 * One entry of a directory listing, its kind read without following a symlink.
 */
export interface DirEntry {
	name: string;
	kind: FileKind;
}

export interface FsStatOptions {
	/**
	 * Whether a final symlink is followed to its target. Defaults to `true`.
	 */
	follow?: boolean;
}

/**
 * The times `set_times` gives an entry, in milliseconds since the epoch, as `FileInfo.mtime_ms`.
 * Each is a finite number from `0` to `8.64e15`, the range of a `Date`, else a `TypeError`,
 * and the filesystem may clamp a time beyond its own range.
 * The Node implementation passes seconds as a double that libuv truncates, to a microsecond
 * on libuv 1.51 (Node 24.14) and to a nanosecond on 1.52.1 (Node 26.13): on the microsecond step an integer is
 * stored exactly through 2242, and on the nanosecond step within half the double's spacing
 * (119ns from 2004 to 2038), so `stat` can show an integer one step of its own double off
 * where that spacing is the coarser, as from 2038-01-19 to 2039-09-07.
 */
export interface FsTimes {
	/**
	 * The last modification time.
	 */
	mtime_ms: number;
	/**
	 * The last access time. Defaults to `mtime_ms`.
	 */
	atime_ms?: number;
}

export interface FsWriteOptions {
	/**
	 * Fail with `'exists'` rather than replace an existing entry.
	 */
	create_new?: boolean;
	/**
	 * The permission bits of a newly created file, before the umask, masked to `0o777`,
	 * so no setuid, setgid, or sticky bit. An integer from `0` to `0o7777`, else a `TypeError`.
	 */
	mode?: number;
}

export interface FsMkdirOptions {
	/**
	 * Create missing parents too; an existing directory is then not an error.
	 */
	recursive?: boolean;
	/**
	 * Return normally when the directory exists. An existing non-directory still throws.
	 */
	exists_ok?: boolean;
}

export interface FsRemoveOptions {
	/**
	 * Remove a directory and everything in it. Without it, only a file,
	 * a symlink, or an empty directory is removed.
	 */
	recursive?: boolean;
	/**
	 * Return normally when nothing is at the path.
	 */
	missing_ok?: boolean;
}

export interface FsCopyOptions {
	/**
	 * Copy a directory and everything in it, symlinks copied as links.
	 */
	recursive?: boolean;
}

/**
 * A read-only handle on a directory. Every path is relative to it.
 */
export interface FsReadDir {
	/**
	 * The directory's absolute real path, for display and as a spawned process's cwd.
	 * It carries no authority: ops take relative paths on the handle.
	 */
	readonly path: string;
	/**
	 * Narrows to a subdirectory, which must exist.
	 * @throws `FsError` - `not_found`, `not_a_directory`, `outside_root`
	 */
	dir(path: string): FsReadDir;
	/**
	 * @returns the entry's info, or `null` when nothing is at `path`
	 * (a path through a file throws `not_a_directory`)
	 */
	stat(path: string, options?: FsStatOptions): Promise<FileInfo | null>;
	/**
	 * Whether an entry is at `path`, following a final symlink.
	 */
	exists(path: string): Promise<boolean>;
	read(path: string): Promise<Uint8Array>;
	/**
	 * Reads a file as UTF-8.
	 */
	read_text(path: string): Promise<string>;
	/**
	 * @returns the file's bytes, or `null` when nothing is at `path`
	 */
	read_if_exists(path: string): Promise<Uint8Array | null>;
	/**
	 * @returns the file's UTF-8 text, or `null` when nothing is at `path`
	 */
	read_text_if_exists(path: string): Promise<string | null>;
	/**
	 * Lists a directory's entries, in no particular order.
	 * An entry whose name isn't UTF-8 fails the listing with `io_error`,
	 * since no string names it.
	 * @param path - defaults to the handle's own directory
	 */
	list(path?: string): AsyncIterable<DirEntry>;
	/**
	 * Reads a symlink's target as stored, never following the link,
	 * so the target may be absolute or lead outside the handle: reading it reaches nothing.
	 * A target that isn't UTF-8 fails with `io_error`, since no string spells it.
	 * @throws `FsError` - `not_found`; `io_error` when the entry isn't a symlink
	 */
	read_link(path: string): Promise<string>;
	stat_sync(path: string, options?: FsStatOptions): FileInfo | null;
	exists_sync(path: string): boolean;
	read_text_sync(path: string): string;
}

/**
 * A read-write handle on a directory. Every path is relative to it.
 */
export interface FsDir extends FsReadDir {
	dir(path: string): FsDir;
	/**
	 * A handle on the same directory without the write ops, at runtime as well as in types.
	 */
	readonly(): FsReadDir;
	/**
	 * Writes a file, replacing one that exists, or with `create_new` only creating one.
	 * @returns with `create_new`, `'created'`, or `'exists'` when an entry was already there
	 */
	write(
		path: string,
		data: Uint8Array | string,
		options: FsWriteOptions & { create_new: true }
	): Promise<'created' | 'exists'>;
	write(
		path: string,
		data: Uint8Array | string,
		options?: FsWriteOptions & { create_new?: false }
	): Promise<void>;
	write(
		path: string,
		data: Uint8Array | string,
		options?: FsWriteOptions
	): Promise<'created' | 'exists' | void>;
	mkdir(path: string, options?: FsMkdirOptions): Promise<void>;
	/**
	 * Removes an entry. A final symlink is removed, not its target,
	 * and with a trailing `/` refused as `not_a_directory`.
	 * The handle's own directory (`''`) is refused with `io_error`.
	 */
	remove(path: string, options?: FsRemoveOptions): Promise<void>;
	/**
	 * Renames an entry, replacing what's at `to` as the OS does. Symlinks are moved, not followed.
	 * The handle's own directory at either end is refused with `io_error`.
	 */
	rename(from: string, to: string): Promise<void>;
	/**
	 * Copies a file, replacing what's at `to`, or with `recursive` a directory tree.
	 */
	copy(from: string, to: string, options?: FsCopyOptions): Promise<void>;
	/**
	 * Sets an entry's modification and access times, `atime_ms` defaulting to `mtime_ms`.
	 * A final symlink is followed, its target confined as any path is,
	 * unless `follow` is `false`, which sets the link's own times.
	 * The runtime's run box, as designed, refuses `follow: false` with `permission_denied`,
	 * since no form of it can be bounded there.
	 * The current time is the caller's `Date.now()`.
	 * @throws `TypeError` - for a time that isn't a finite number from `0` to `8.64e15`
	 * @throws `FsError` - `not_found`, `not_a_directory`, `permission_denied`, `outside_root`
	 */
	set_times(path: string, times: FsTimes, options?: FsStatOptions): Promise<void>;
	/**
	 * Sets an entry's permission bits, like `0o755`, following a final symlink,
	 * whose target is confined as any path is (Linux has no mode of its own for a link).
	 * The mode is absolute, never symbolic, and holds no setuid, setgid, or sticky bit.
	 * @throws `TypeError` - for a mode that isn't an integer from `0` to `0o777`
	 * @throws `FsError` - `not_found`, `not_a_directory`, `permission_denied`, `outside_root`
	 */
	set_mode(path: string, mode: number): Promise<void>;
	/**
	 * Creates a symlink at `path` pointing at `target`, path first like every handle op.
	 * The target is data, stored verbatim and never resolved, so it may be relative,
	 * absolute, or dangling: following a link is what's confined, by every handle that reaches it.
	 * The final component of `path` is never followed, and its parent is confined.
	 * @returns `'created'`, or `'exists'` when any entry was already at `path`, a dangling
	 * symlink included, which is left untouched
	 * @throws `TypeError` - for a `path` ending in `/`, `/.`, or `/..`, which names a directory,
	 * unless it resolves to the handle's own directory (`''`, `'.'`, `'d/..'`), an existing entry
	 * and so `'exists'`; or a `target` that is empty, holds a NUL byte, or isn't well-formed Unicode
	 * @throws `FsError` - `not_found` for a missing parent, `not_a_directory`, `permission_denied`,
	 * `outside_root`
	 */
	symlink(path: string, target: string): Promise<'created' | 'exists'>;
}

/**
 * The fs host module's root: the project directory and the ways to reach others.
 */
export interface Fs {
	/**
	 * The project directory.
	 */
	readonly project: FsDir;
	/**
	 * Opens a directory by absolute path.
	 * @throws `TypeError` - for a relative path, or one holding a NUL byte or a lone surrogate
	 * @throws `FsError` - `not_found`, `not_a_directory`
	 */
	open_dir(path: string): Promise<FsDir>;
	/**
	 * Creates a fresh directory under the OS temp directory,
	 * removed with everything in it when disposed (`await using`).
	 * @param prefix - the start of the directory's name, defaults to `'gro-'`.
	 * It names no other directory: a non-string, a `/`, or a NUL byte throws `TypeError`,
	 * and `..` is only the name's start (`..` then six random characters).
	 * @throws `TypeError` - for a prefix that isn't a string or holds a `/` or a NUL byte
	 */
	temp_dir(prefix?: string): Promise<FsDir & AsyncDisposable>;
}

// the most symlinks one path resolution follows, Linux's own limit
const MAX_SYMLINKS = 40;

// the most bytes Linux takes in a path, its NUL included
const PATH_MAX = 4096;

const to_kind = (code: unknown): FsErrorKind => {
	switch (code) {
		case 'ENOENT':
			return 'not_found';
		case 'EEXIST':
			return 'already_exists';
		case 'EACCES':
		case 'EPERM':
			return 'permission_denied';
		case 'ENOTDIR':
			return 'not_a_directory';
		case 'EISDIR':
		case 'ERR_FS_EISDIR':
			return 'is_a_directory';
		case 'ENOTEMPTY':
			return 'not_empty';
		default:
			return 'io_error';
	}
};

const code_of = (error: unknown): unknown =>
	typeof error === 'object' && error !== null ? (error as { code?: unknown }).code : undefined;

const is_not_found = (error: unknown): boolean =>
	code_of(error) === 'ENOENT' || (is_own_fs_error(error) && error.kind === 'not_found');

/**
 * Classifies a Node error by its code, naming `path`, the path the op reports.
 * An `FsError` passes through when it already names `path` or is `outside_root`,
 * which names the path as requested; one from inside a walk, naming a path the walk
 * resolved, is rewrapped to name `path`, the resolved one kept in its message and `cause`.
 */
const to_fs_error = (error: unknown, path: string): FsError => {
	if (is_own_fs_error(error)) {
		if (error.path === path || error.kind === 'outside_root') return error;
		return new FsError(error.kind, path, `${error.kind}: ${path} (${error.message})`, {
			cause: error
		});
	}
	const kind = to_kind(code_of(error));
	const detail = error instanceof Error ? error.message : String(error);
	return new FsError(kind, path, `${kind}: ${path} (${detail})`, { cause: error });
};

const to_file_kind = (s: Stats | Dirent<string | Buffer>): FileKind =>
	s.isFile() ? 'file' : s.isDirectory() ? 'directory' : s.isSymbolicLink() ? 'symlink' : 'other';

const to_file_info = (s: Stats): FileInfo => ({
	kind: to_file_kind(s),
	size: s.size,
	mtime_ms: s.mtimeMs,
	mode: s.mode & 0o7777
});

// a plain `Uint8Array` view of a Node `Buffer`, so code can't come to rely on `Buffer` methods
const to_bytes = (buffer: Uint8Array): Uint8Array =>
	new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);

/**
 * A directory's real path and what a path must start with to be inside it.
 */
interface Root {
	path: string;
	prefix: string;
}

const to_root = (real_path: string): Root => ({
	path: real_path,
	prefix: real_path === '/' ? '/' : real_path + '/'
});

/**
 * A path requested beneath a handle, resolved lexically.
 */
interface LexicalPath {
	/**
	 * The absolute path, its `.` and `..` segments resolved, with no trailing `/`.
	 */
	path: string;
	/**
	 * The segments beneath the root, none of them `.` or `..`.
	 */
	segments: Array<string>;
	/**
	 * Whether the request ended in `/`, `/.`, or `/..`, so the entry must be a directory,
	 * as the kernel reads a trailing `/`.
	 */
	dir: boolean;
}

/**
 * Refuses a path that isn't a string, holds a NUL byte, or isn't well-formed Unicode,
 * whose lone surrogates would encode as U+FFFD and name a different entry than it spells.
 */
const assert_path_string = (path: string): void => {
	if (typeof path !== 'string') throw new TypeError('fs paths are strings');
	if (path.includes('\0')) throw new TypeError('fs paths hold no NUL byte');
	if (!path.isWellFormed()) throw new TypeError('fs paths are well-formed Unicode');
};

// the latest time a `Date` holds, in milliseconds since the epoch
const MAX_TIME_MS = 8.64e15;

/**
 * Checks a time for `set_times`, a finite number from `0` to `MAX_TIME_MS`. Node reads a
 * negative time as now, so none reaches it.
 */
const to_time_ms = (value: unknown, name: string): number => {
	if (typeof value !== 'number' || !(value >= 0 && value <= MAX_TIME_MS)) {
		throw new TypeError(`fs times are finite numbers of ms from 0 to ${MAX_TIME_MS}: ${name}`);
	}
	return value;
};

/**
 * Half the step the seconds Node's `utimes` takes are truncated to, in milliseconds.
 * Node's libuv truncates to a microsecond on 1.51 and to a nanosecond on 1.52.1, and any
 * libuv below 1.52 is taken as the microsecond step; Bun and Deno report a libuv version but
 * set times without it, Bun to the nanosecond and Deno to the millisecond.
 */
const UTIMES_HALF_STEP_MS = ((): number => {
	if (versions.bun !== undefined || versions.deno !== undefined) return 5e-7;
	const [major = 0, minor = 0] = (versions.uv ?? '').split('.').map(Number);
	return major === 1 && minor < 52 ? 5e-4 : 5e-7;
})();

/**
 * Milliseconds as the seconds Node's `utimes` takes, aimed half a step past the time so the
 * truncation lands on it: `1700000000123 / 1000`, a double 94ns short, would store
 * `…122.999` where libuv truncates to microseconds. The whole seconds are split off first,
 * so the sum rounds once, to the double nearest the aim.
 */
const to_unix_seconds = (ms: number): number => {
	let seconds = Math.trunc(ms / 1000);
	let rest_ms = ms - seconds * 1000;
	if (rest_ms < 0) {
		seconds -= 1;
		rest_ms += 1000;
	}
	return seconds + (rest_ms + UTIMES_HALF_STEP_MS) / 1000;
};

/**
 * Checks a permission mode for `set_mode`, an integer from `0` to `0o777`,
 * so no setuid, setgid, or sticky bit, and no string Node would parse as octal.
 */
const to_permission_mode = (mode: unknown): number => {
	if (typeof mode !== 'number' || !Number.isInteger(mode) || mode < 0 || mode > 0o777) {
		throw new TypeError('fs modes are integers from 0 to 0o777');
	}
	// `-0` as `0`, since Node 24's `chmod` binding aborts the process on a mode that isn't an int32
	return mode | 0;
};

/**
 * Checks `write`'s mode, an integer from `0` to `0o7777`, and masks it to `0o777`,
 * as the runtime's builtin does, so no write creates a setuid, setgid, or sticky file
 * and no string reaches Node, which would parse it as octal.
 */
const to_write_mode = (mode: unknown): number => {
	if (typeof mode !== 'number' || !Number.isInteger(mode) || mode < 0 || mode > 0o7777) {
		throw new TypeError('fs write modes are integers from 0 to 0o7777');
	}
	return mode & 0o777;
};

/**
 * Checks a symlink's target, stored verbatim: a non-empty string without a NUL byte
 * or a lone surrogate, which would encode as U+FFFD and store a different target than it spells.
 */
const assert_link_target = (target: string): void => {
	if (typeof target !== 'string') throw new TypeError('fs symlink targets are strings');
	if (target === '') throw new TypeError('fs symlink targets are not empty');
	if (target.includes('\0')) throw new TypeError('fs symlink targets hold no NUL byte');
	if (!target.isWellFormed()) throw new TypeError('fs symlink targets are well-formed Unicode');
};

/**
 * Refuses a path segment holding a `/`, which no split of a path makes, so a segment forged
 * by a polluted intrinsic (`split`, `pop`, an array iterator) is never taken as one component.
 * Checked by index, which no prototype can intercept.
 */
const assert_segment = (name: unknown, requested: string): string => {
	if (typeof name !== 'string') {
		throw to_outside_error(requested, 'yields a path segment that is not a string');
	}
	for (let i = 0; i < name.length; i++) {
		if (name[i] === '/') {
			throw to_outside_error(requested, `yields the path segment ${name}, which holds a \`/\``);
		}
	}
	return name;
};

/**
 * Resolves `requested` lexically beneath `root`, refusing an absolute path
 * and one whose `..` segments climb above it.
 * @throws `TypeError` - for a path that isn't a well-formed string or holds a NUL byte
 * @throws `FsError` - `outside_root`
 */
const to_lexical_path = (root: Root, requested: string): LexicalPath => {
	assert_path_string(requested);
	if (requested.startsWith('/')) {
		throw new FsError('outside_root', requested, `outside_root: absolute path ${requested}`);
	}
	const parts = requested.split('/');
	const segments: Array<string> = [];
	for (const segment of parts) {
		if (segment === '' || segment === '.') continue;
		if (segment === '..') {
			if (segments.length === 0) {
				throw new FsError(
					'outside_root',
					requested,
					`outside_root: ${requested} climbs above ${root.path}`
				);
			}
			segments.pop();
		} else {
			segments.push(assert_segment(segment, requested));
		}
	}
	if (segments.length === 0) return { path: root.path, segments, dir: false };
	const last = parts.at(-1);
	return {
		path: root.prefix + segments.join('/'),
		segments,
		dir: last === '' || last === '.' || last === '..'
	};
};

// the path an op runs on, keeping a requested trailing `/` so the kernel requires a directory
const to_op_path = (target: string, lexical: LexicalPath): string =>
	lexical.dir ? target + '/' : target;

const to_root_error = (lexical: LexicalPath, verb: string): FsError =>
	new FsError(
		'io_error',
		lexical.path,
		`io_error: the handle's own directory can't be ${verb}: ${lexical.path}`
	);

/*

Confinement, matching `openat2` with `RESOLVE_BENEATH`. The requested path's `..` segments are
resolved lexically first (`to_lexical_path`), then the path is walked from the handle's root
one component at a time: a symlink is read and its target walked in its place, an absolute
target is `outside_root`, and a `..` in a target that would climb above the root is
`outside_root`, even when the path would land back inside. With `follow` the final component
is followed too; without it the walk stops at the final component, for ops that act on a
symlink itself.

The walk returns the path the op should use: the real path of an existing entry, so the op
follows no symlink the walk didn't see, or for a missing one its real parent joined with the
rest. A final symlink whose target is missing returns the link's own path, so the op follows it
as the kernel would (`create_new` then reports `'exists'`, as `O_EXCL` does).

A component the walk passes through must be a directory, as the kernel requires, so a
symlink to `a.txt/` or `a.txt/..` is `not_a_directory`, never the file or the root.

Async confinement first tries `realpath`: a path that is its own real path holds no symlink,
which settles the common case in one call. A missing final component whose parent is its own real
path, and that `lstat` finds no symlink at, settles in two more, so a miss skips the walk.
When an async op passes `existing`, nothing being at the path is `not_found` at once, without
the final op, unless the path may be too long for the kernel, which the final op then reports;
a sync miss constructs no error to spare, so the sync ops don't.

The walk is synchronous, in the async ops too, so its `lstat`s and `readlink`s don't each cost a
trip through the libuv threadpool. An async op's walk runs after `realpath` has just resolved
the same components, which leaves them in the kernel's dentry cache, so it blocks the event
loop only for those lookups, and on a network or FUSE filesystem for however long the server
takes to revalidate them, bounded by the path's components and `MAX_SYMLINKS`.

A real path beneath the root's prefix can't have come through a symlink in the root's own
path, since a real path holds no symlink, so the walk skips the root re-check after one.
Reads use `lstatSync` with `throwIfNoEntry: false`, so a missing entry constructs no error.

*/

const to_outside_error = (requested: string, detail: string): FsError =>
	new FsError('outside_root', requested, `outside_root: ${requested} ${detail}`);

const to_moved_root_error = (root: Root, requested: string, root_real: string): FsError =>
	to_outside_error(requested, `resolves through the moved ${root.path} to ${root_real}`);

const to_absolute_link_error = (requested: string, link: string, target: string): FsError =>
	to_outside_error(
		requested,
		`reaches the absolute symlink ${link} -> ${target}, which a handle never follows`
	);

const to_climb_error = (root: Root, requested: string, link: string | null): FsError =>
	to_outside_error(
		requested,
		link === null
			? `climbs above ${root.path}`
			: `climbs above ${root.path} through the symlink ${link}`
	);

const to_symlinks_error = (path: string): FsError =>
	new FsError('io_error', path, `io_error: too many symlinks at ${path}`);

/**
 * The state of one walk beneath a root. Segments are popped from the end of `stack`.
 */
interface Walk {
	current: string;
	stack: Array<string>;
	links: number;
	// the last symlink whose target the walk entered, for the climb error
	last_link: string | null;
	// a symlink met as the final component, returned when its target is missing
	final_link: string | null;
}

const create_walk = (root: Root, segments: Array<string>): Walk => ({
	current: root.path,
	stack: segments.toReversed(),
	links: 0,
	last_link: null,
	final_link: null
});

/**
 * Takes the walk's next segment, applying `.` and `..`.
 * @returns the next component's path, or `null` when the segment needs no lookup
 */
const step_walk = (root: Root, requested: string, walk: Walk): string | null => {
	const name = assert_segment(walk.stack.pop(), requested);
	if (name === '' || name === '.') return null;
	if (name === '..') {
		if (walk.current === root.path) throw to_climb_error(root, requested, walk.last_link);
		walk.current = dirname(walk.current);
		return null;
	}
	return walk.current === '/' ? '/' + name : walk.current + '/' + name;
};

/**
 * The path for a component the walk found missing: the rest of the path can't hold a symlink.
 */
const to_missing_path = (walk: Walk, next: string): string => {
	if (walk.stack.includes('..')) {
		// a symlink target climbing out of a missing directory, which the kernel can't walk either
		throw new FsError('not_found', next);
	}
	if (walk.final_link !== null) return walk.final_link;
	return walk.stack.length === 0 ? next : next + '/' + walk.stack.toReversed().join('/');
};

// fatal, so bytes that aren't UTF-8 throw rather than decode to U+FFFD,
// and with `ignoreBOM`, so a leading BOM is kept as part of the name
const utf8_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/**
 * Decodes a symlink target or entry name read as bytes, refusing one that isn't UTF-8:
 * decoded lossily, its U+FFFD would name a different entry than the kernel follows.
 * The refusal is `io_error`, not `outside_root`, since nothing showed the bytes leave the
 * handle; the runtime's native op, which never decodes them, follows such a link.
 */
const decode_name = (bytes: Uint8Array, path: string, what: string): string => {
	try {
		return utf8_decoder.decode(bytes);
	} catch {
		throw new FsError(
			'io_error',
			path,
			`io_error: ${path} ${what} ${to_escaped_bytes(bytes)}, which isn't UTF-8`
		);
	}
};

/**
 * A path's real path, read as bytes and decoded as UTF-8, so a real path that isn't
 * fails with `io_error` rather than decoding to U+FFFD and naming a different path.
 */
const realpath_text = async (path: string): Promise<string> =>
	decode_name(await realpath(path, { encoding: 'buffer' }), path, 'has the real path');

const realpath_text_sync = (path: string): string =>
	decode_name(realpath_sync_native(path, { encoding: 'buffer' }), path, 'has the real path');

// printable ASCII as itself, every other byte as `\xNN`
const to_escaped_bytes = (bytes: Uint8Array): string => {
	let text = '';
	for (const byte of bytes) {
		text +=
			byte >= 0x20 && byte < 0x7f && byte !== 0x5c
				? String.fromCharCode(byte)
				: '\\x' + byte.toString(16).padStart(2, '0');
	}
	return text;
};

/**
 * Takes a symlink's target into the walk in place of the link.
 * The target is read as bytes and must be UTF-8, so the walk follows the bytes the kernel would.
 */
const enter_link = (
	requested: string,
	walk: Walk,
	link: string,
	target_bytes: Uint8Array
): void => {
	if (++walk.links > MAX_SYMLINKS) throw to_symlinks_error(link);
	const target = decode_name(target_bytes, link, 'is a symlink to');
	if (target.startsWith('/')) throw to_absolute_link_error(requested, link, target);
	if (walk.stack.length === 0) walk.final_link ??= link;
	walk.last_link = link;
	const parts = target.split('/');
	for (let i = parts.length - 1; i >= 0; i--) walk.stack.push(parts[i]!);
};

/**
 * Whether nothing is at `path` and its parent holds no symlink, so the path is inside any
 * root it lexically is: a parent that is its own real path has no symlink to leave through,
 * and `lstat` finds a final symlink, dangling or not, which the walk must see.
 * Synchronous, since `realpath` just failed on these components and left them cached.
 */
const is_missing_beneath_real_parent = (path: string): boolean => {
	const parent = dirname(path);
	try {
		return (
			realpath_text_sync(parent) === parent &&
			lstatSync(path, { throwIfNoEntry: false }) === undefined
		);
	} catch {
		return false;
	}
};

/**
 * Whether the kernel can't refuse `path` as too long, a requested trailing `/` and the NUL
 * counted. `path` is the whole op path, root included, and always well-formed text whose
 * UTF-8 bytes are what the op passes, at most 3 per UTF-16 code unit. Checked before answering
 * a miss without the final op, which alone would report `ENAMETOOLONG`.
 */
const is_within_path_max = (path: string): boolean => path.length * 3 + 2 <= PATH_MAX;

/**
 * The walk, once the fast path hasn't settled the path. Synchronous for sync and async ops alike.
 * @param real - the real path the fast path found, or `null` when it failed
 * @param existing - see `confine`
 */
const confine_walk = (
	root: Root,
	requested: string,
	lexical: LexicalPath,
	follow: boolean,
	real: string | null,
	existing: boolean
): string => {
	// a real path beneath the prefix holds no symlink, so the root's own path has none
	if (real === null || !real.startsWith(root.prefix)) {
		const root_real = realpath_text_sync(root.path);
		if (root_real !== root.path) throw to_moved_root_error(root, requested, root_real);
	}
	const walk = create_walk(root, lexical.segments);
	while (walk.stack.length > 0) {
		const next = step_walk(root, requested, walk);
		if (next === null) continue;
		if (!follow && walk.stack.length === 0) return next;
		let s: Stats | undefined;
		try {
			// `undefined` when missing; `lstatSync` throws `ENOTDIR` instead
			s = lstatSync(next, { throwIfNoEntry: false });
		} catch (error) {
			if (code_of(error) === 'ENOTDIR') return to_missing_path(walk, next);
			throw error;
		}
		if (s === undefined) {
			const missing = to_missing_path(walk, next);
			if (existing && is_within_path_max(missing)) throw new FsError('not_found', lexical.path);
			return missing;
		}
		if (s.isSymbolicLink()) {
			enter_link(requested, walk, next, readlinkSync(next, { encoding: 'buffer' }));
		} else {
			// a component the walk passes through must be a directory, as the kernel requires
			if (walk.stack.length > 0 && !s.isDirectory()) throw new FsError('not_a_directory', next);
			walk.current = next;
		}
	}
	return walk.current;
};

/**
 * Finds the path beneath `root` an op should run on.
 * @param existing - the op needs an entry at the path, so a miss throws `not_found`
 * here rather than returning a path for the op to fail on, sparing the final op
 * when the path is short enough that the op couldn't fail another way
 */
const confine = async (
	root: Root,
	requested: string,
	lexical: LexicalPath,
	follow: boolean,
	existing = false
): Promise<string> => {
	const { path } = lexical;
	const head = follow || path === root.path ? path : dirname(path);
	let real: string | null = null;
	try {
		real = await realpath_text(head);
		if (real === head) return path;
	} catch (error) {
		// missing, or not a directory, which the walk sorts out, but for the common miss
		if (
			code_of(error) === 'ENOENT' &&
			head === path &&
			path !== root.path &&
			is_missing_beneath_real_parent(path)
		) {
			if (existing && is_within_path_max(path)) throw new FsError('not_found', path);
			return path;
		}
	}
	return confine_walk(root, requested, lexical, follow, real, existing);
};

/**
 * The sync twin of `confine`, always walking. A `realpath` fast path here constructs an
 * error on every miss, which measured costlier than the `lstat`s it saves on a hit.
 */
const confine_sync = (
	root: Root,
	requested: string,
	lexical: LexicalPath,
	follow: boolean
): string => confine_walk(root, requested, lexical, follow, null, false);

/**
 * Why a sync stat found nothing at `target`, a path a walk returned. Node's `statSync`
 * with `throwIfNoEntry: false` folds `ENOTDIR` into absent (`a.txt/`, or a path through
 * a file), so this stats the path's ancestors, on the missing path only,
 * for the first one that exists.
 */
const to_missing_kind_sync = (
	root: Root,
	target: string,
	dir: boolean
): 'not_found' | 'not_a_directory' => {
	// with a trailing `/`, the entry itself may be the non-directory
	let path = dir ? target : dirname(target);
	for (;;) {
		const s = statSync(path, { throwIfNoEntry: false });
		if (s !== undefined) return s.isDirectory() ? 'not_found' : 'not_a_directory';
		if (path.length <= root.path.length) return 'not_found';
		path = dirname(path);
	}
};

const stat_beneath = async (
	root: Root,
	path: string,
	options?: FsStatOptions
): Promise<FileInfo | null> => {
	const lexical = to_lexical_path(root, path);
	// a trailing `/` follows a final symlink, as in the kernel
	const follow = (options?.follow ?? true) || lexical.dir;
	try {
		const target = to_op_path(await confine(root, path, lexical, follow, true), lexical);
		return to_file_info(await (follow ? stat(target) : lstat(target)));
	} catch (error) {
		if (is_not_found(error)) return null;
		throw to_fs_error(error, lexical.path);
	}
};

const stat_sync_beneath = (root: Root, path: string, options?: FsStatOptions): FileInfo | null => {
	const lexical = to_lexical_path(root, path);
	const follow = (options?.follow ?? true) || lexical.dir;
	let target: string;
	let s: Stats | undefined;
	try {
		target = confine_sync(root, path, lexical, follow);
		const op_path = to_op_path(target, lexical);
		s = follow
			? statSync(op_path, { throwIfNoEntry: false })
			: lstatSync(op_path, { throwIfNoEntry: false });
	} catch (error) {
		if (is_not_found(error)) return null;
		throw to_fs_error(error, lexical.path);
	}
	if (s !== undefined) return to_file_info(s);
	const kind = to_missing_kind_sync(root, target, lexical.dir);
	if (kind === 'not_found') return null;
	throw new FsError(kind, lexical.path);
};

/**
 * Resolves `path` beneath `root` as every op does, following a final symlink, for another host
 * module holding the handle (`internal/fs_handles.ts`).
 * @throws `TypeError` - for a path that isn't a well-formed string or holds a NUL byte
 * @throws `FsError` - `outside_root`, or the kind the walk met, naming the requested path
 */
const resolve_beneath_sync = (root: Root, path: string): FsHandleResolution => {
	const lexical = to_lexical_path(root, path);
	try {
		const target = confine_sync(root, path, lexical, true);
		return Object.freeze({ found_path: lexical.path, op_path: to_op_path(target, lexical) });
	} catch (error) {
		throw to_fs_error(error, lexical.path);
	}
};

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('fs handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) {
		throw new TypeError('fs handles come from `fs` and other handles, not a constructor');
	}
};

class NodeFsReadDir implements FsReadDir {
	readonly path: string;
	readonly #root: Root;

	constructor(key: symbol, real_path: string) {
		check_handle_key(key);
		this.path = real_path;
		const root = to_root(real_path);
		this.#root = root;
		// every handle class runs this constructor, so every handle is registered
		register_fs_handle(this, (path) => resolve_beneath_sync(root, path));
	}

	dir(path: string): FsReadDir {
		return create_read_dir(open_subdir(this.#root, path));
	}

	async stat(path: string, options?: FsStatOptions): Promise<FileInfo | null> {
		return stat_beneath(this.#root, path, options);
	}

	async exists(path: string): Promise<boolean> {
		return (await stat_beneath(this.#root, path)) !== null;
	}

	async read(path: string): Promise<Uint8Array> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			const target = await confine(this.#root, path, lexical, true);
			return to_bytes(await readFile(to_op_path(target, lexical)));
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	async read_text(path: string): Promise<string> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			const target = await confine(this.#root, path, lexical, true);
			return await readFile(to_op_path(target, lexical), 'utf8');
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	async read_if_exists(path: string): Promise<Uint8Array | null> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			const target = await confine(this.#root, path, lexical, true, true);
			return to_bytes(await readFile(to_op_path(target, lexical)));
		} catch (error) {
			if (is_not_found(error)) return null;
			throw to_fs_error(error, lexical.path);
		}
	}

	async read_text_if_exists(path: string): Promise<string | null> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			const target = await confine(this.#root, path, lexical, true, true);
			return await readFile(to_op_path(target, lexical), 'utf8');
		} catch (error) {
			if (is_not_found(error)) return null;
			throw to_fs_error(error, lexical.path);
		}
	}

	async *list(path = ''): AsyncIterable<DirEntry> {
		const lexical = to_lexical_path(this.#root, path);
		let entries: Dir;
		try {
			// names read as bytes, so one that isn't UTF-8 is refused rather than listed as a
			// lossy name that reaches a different entry, or none (Node's types omit `'buffer'`)
			entries = await opendir(await confine(this.#root, path, lexical, true), {
				encoding: 'buffer' as BufferEncoding
			});
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
		try {
			for await (const entry of entries as AsyncIterable<Dirent<Buffer>>) {
				yield {
					name: decode_name(entry.name, lexical.path, 'holds an entry named'),
					kind: to_file_kind(entry)
				};
			}
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	async read_link(path: string): Promise<string> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			// the final link is read, not followed; with a trailing `/` the kernel follows it,
			// so the walk does, and what it reaches is a directory, no symlink to read
			const target = await confine(this.#root, path, lexical, lexical.dir, true);
			const bytes = await readlink(to_op_path(target, lexical), { encoding: 'buffer' });
			return decode_name(bytes, lexical.path, 'is a symlink to');
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	stat_sync(path: string, options?: FsStatOptions): FileInfo | null {
		return stat_sync_beneath(this.#root, path, options);
	}

	exists_sync(path: string): boolean {
		return stat_sync_beneath(this.#root, path) !== null;
	}

	read_text_sync(path: string): string {
		const lexical = to_lexical_path(this.#root, path);
		try {
			const target = confine_sync(this.#root, path, lexical, true);
			return readFileSync(to_op_path(target, lexical), 'utf8');
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}
}

// opens a subdirectory of `root` synchronously, the narrowing `dir` does
const open_subdir = (root: Root, path: string): string => {
	const lexical = to_lexical_path(root, path);
	try {
		const real_path = confine_sync(root, path, lexical, true);
		const s = statSync(real_path, { throwIfNoEntry: false });
		if (s === undefined) {
			throw new FsError(to_missing_kind_sync(root, real_path, false), lexical.path);
		}
		if (!s.isDirectory()) throw new FsError('not_a_directory', lexical.path);
		return real_path;
	} catch (error) {
		throw to_fs_error(error, lexical.path);
	}
};

// the write ops keep their own private root, so they can't be called on a read-only handle
class NodeFsDir extends NodeFsReadDir implements FsDir {
	readonly #root: Root;

	constructor(key: symbol, real_path: string) {
		super(key, real_path);
		this.#root = to_root(real_path);
	}

	override dir(path: string): FsDir {
		return create_dir(open_subdir(this.#root, path));
	}

	readonly(): FsReadDir {
		return create_read_dir(this.#root.path);
	}

	write(
		path: string,
		data: Uint8Array | string,
		options: FsWriteOptions & { create_new: true }
	): Promise<'created' | 'exists'>;
	write(
		path: string,
		data: Uint8Array | string,
		options?: FsWriteOptions & { create_new?: false }
	): Promise<void>;
	write(
		path: string,
		data: Uint8Array | string,
		options?: FsWriteOptions
	): Promise<'created' | 'exists' | void>;
	async write(
		path: string,
		data: Uint8Array | string,
		options?: FsWriteOptions
	): Promise<'created' | 'exists' | void> {
		const create_new = options?.create_new ?? false;
		const lexical = to_lexical_path(this.#root, path);
		const requested_mode = options?.mode;
		const mode = requested_mode === undefined ? undefined : to_write_mode(requested_mode);
		try {
			const target = await confine(this.#root, path, lexical, true);
			await writeFile(to_op_path(target, lexical), data, {
				flag: create_new ? 'wx' : 'w',
				mode
			});
		} catch (error) {
			if (create_new && code_of(error) === 'EEXIST') return 'exists';
			throw to_fs_error(error, lexical.path);
		}
		if (create_new) return 'created';
	}

	async mkdir(path: string, options?: FsMkdirOptions): Promise<void> {
		const lexical = to_lexical_path(this.#root, path);
		try {
			await mkdir(await confine(this.#root, path, lexical, true), {
				recursive: options?.recursive
			});
		} catch (error) {
			if (options?.exists_ok && code_of(error) === 'EEXIST') {
				// only an existing directory is ok, which costs a `stat` on this path alone
				const s = await stat_beneath(this.#root, path);
				if (s?.kind === 'directory') return;
			}
			throw to_fs_error(error, lexical.path);
		}
	}

	async remove(path: string, options?: FsRemoveOptions): Promise<void> {
		const lexical = to_lexical_path(this.#root, path);
		if (lexical.path === this.#root.path) throw to_root_error(lexical, 'removed');
		try {
			const target = await confine(this.#root, path, lexical, false);
			// a trailing `/` removes only a directory, never a symlink, as `rmdir` and `unlink` do
			if (lexical.dir && !(await lstat(target)).isDirectory()) {
				throw new FsError('not_a_directory', lexical.path);
			}
			if (options?.recursive) {
				await rm(target, { recursive: true });
			} else {
				try {
					await unlink(target);
				} catch (error) {
					// Linux's `unlink` refuses a directory with `EISDIR`, other systems with `EPERM`
					const code = code_of(error);
					if (code !== 'EISDIR' && code !== 'EPERM') throw error;
					if ((await lstat(target)).isDirectory()) await rmdir(target);
					else throw error;
				}
			}
		} catch (error) {
			if (options?.missing_ok && is_not_found(error)) return;
			throw to_fs_error(error, lexical.path);
		}
	}

	async rename(from: string, to: string): Promise<void> {
		const from_lexical = to_lexical_path(this.#root, from);
		const to_lexical = to_lexical_path(this.#root, to);
		if (from_lexical.path === this.#root.path) throw to_root_error(from_lexical, 'renamed');
		if (to_lexical.path === this.#root.path) throw to_root_error(to_lexical, 'replaced');
		const source = await confine_end(this.#root, from, from_lexical, false);
		const target = await confine_end(this.#root, to, to_lexical, false);
		try {
			// a trailing `/` on either end makes the kernel require a directory, never a symlink
			await rename(to_op_path(source, from_lexical), to_op_path(target, to_lexical));
		} catch (error) {
			throw await to_two_path_error(error, source, from_lexical, to_lexical);
		}
	}

	async copy(from: string, to: string, options?: FsCopyOptions): Promise<void> {
		const from_lexical = to_lexical_path(this.#root, from);
		const to_lexical = to_lexical_path(this.#root, to);
		const source = await confine_end(this.#root, from, from_lexical, true);
		const target = await confine_end(this.#root, to, to_lexical, true);
		try {
			const source_path = to_op_path(source, from_lexical);
			const target_path = to_op_path(target, to_lexical);
			if (options?.recursive) {
				await cp(source_path, target_path, { recursive: true, verbatimSymlinks: true });
			} else {
				await copyFile(source_path, target_path);
			}
		} catch (error) {
			throw await to_two_path_error(error, source, from_lexical, to_lexical);
		}
	}

	async set_times(path: string, times: FsTimes, options?: FsStatOptions): Promise<void> {
		const lexical = to_lexical_path(this.#root, path);
		// each read once, so a getter can't answer the check and the op differently
		const mtime_ms = to_time_ms(times.mtime_ms, 'mtime_ms');
		const atime = times.atime_ms;
		const atime_ms = atime === undefined ? mtime_ms : to_time_ms(atime, 'atime_ms');
		// a trailing `/` follows a final symlink, as in the kernel
		const follow = (options?.follow ?? true) || lexical.dir;
		try {
			const target = to_op_path(await confine(this.#root, path, lexical, follow, true), lexical);
			await (follow ? utimes : lutimes)(
				target,
				to_unix_seconds(atime_ms),
				to_unix_seconds(mtime_ms)
			);
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	async set_mode(path: string, mode: number): Promise<void> {
		const lexical = to_lexical_path(this.#root, path);
		const checked_mode = to_permission_mode(mode);
		try {
			const target = await confine(this.#root, path, lexical, true, true);
			await chmod(to_op_path(target, lexical), checked_mode);
		} catch (error) {
			throw to_fs_error(error, lexical.path);
		}
	}

	async symlink(path: string, target: string): Promise<'created' | 'exists'> {
		const lexical = to_lexical_path(this.#root, path);
		assert_link_target(target);
		// the kernel reads a trailing `/` as a directory to follow, never a name to create
		if (lexical.dir) throw new TypeError(`fs.symlink takes a path with no trailing \`/\`: ${path}`);
		try {
			// the parent is confined and the final component never followed, so whatever is
			// there, a dangling link included, makes the kernel's `EEXIST`, the only check
			await symlink(target, await confine(this.#root, path, lexical, false));
		} catch (error) {
			if (code_of(error) === 'EEXIST') return 'exists';
			throw to_fs_error(error, lexical.path);
		}
		return 'created';
	}
}

// confines one end of a two-path op, its errors naming that end
const confine_end = async (
	root: Root,
	requested: string,
	lexical: LexicalPath,
	follow: boolean
): Promise<string> => {
	try {
		return await confine(root, requested, lexical, follow);
	} catch (error) {
		throw to_fs_error(error, lexical.path);
	}
};

/**
 * Classifies a two-path op's failure. A missing entry or a non-directory could be
 * either end's, so on those alone this checks whether the source is there,
 * and names the source when it isn't, else the destination.
 */
const to_two_path_error = async (
	error: unknown,
	source: string,
	from_lexical: LexicalPath,
	to_lexical: LexicalPath
): Promise<FsError> => {
	const code = code_of(error);
	let path = from_lexical.path;
	if (code === 'ENOENT' || code === 'ENOTDIR') {
		const source_exists = await lstat(to_op_path(source, from_lexical)).then(
			() => true,
			() => false
		);
		if (source_exists) path = to_lexical.path;
	}
	const kind = to_kind(code);
	const detail = error instanceof Error ? error.message : String(error);
	return new FsError(
		kind,
		path,
		`${kind}: ${path} (${from_lexical.path} -> ${to_lexical.path}: ${detail})`,
		{ cause: error }
	);
};

class NodeFsTempDir extends NodeFsDir implements AsyncDisposable {
	readonly #path: string;

	constructor(key: symbol, real_path: string) {
		super(key, real_path);
		this.#path = real_path;
	}

	async [Symbol.asyncDispose](): Promise<void> {
		await rm(this.#path, { recursive: true, force: true });
	}
}

// null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value,
// the subclasses through this base, and frozen, so one holder of a handle can't patch the ops
// another holder calls
Object.setPrototypeOf(NodeFsReadDir.prototype, null);
for (const handle_class of [NodeFsReadDir, NodeFsDir, NodeFsTempDir]) {
	Object.freeze(handle_class.prototype);
	Object.freeze(handle_class);
}

const create_read_dir = (real_path: string): FsReadDir =>
	Object.freeze(new NodeFsReadDir(HANDLE_KEY, real_path));

const create_dir = (real_path: string): FsDir =>
	Object.freeze(new NodeFsDir(HANDLE_KEY, real_path));

// frozen and null-prototype, as the handles are, so awaiting `fs` adopts no polluted `then`
/**
 * The fs host module's root handle. Import it where authority enters
 * (a task, a script, an entry module) and hand narrower handles down.
 */
export const fs: Fs = Object.freeze(
	create_record<Fs>({
		project: create_dir(to_checked_cwd()),
		open_dir: async (path: string): Promise<FsDir> => {
			assert_path_string(path);
			if (!isAbsolute(path)) throw new TypeError(`fs.open_dir takes an absolute path: ${path}`);
			try {
				const real_path = await realpath_text(path);
				if (!(await stat(real_path)).isDirectory()) throw new FsError('not_a_directory', path);
				return create_dir(real_path);
			} catch (error) {
				throw to_fs_error(error, path);
			}
		},
		temp_dir: async (prefix = 'gro-'): Promise<FsDir & AsyncDisposable> => {
			// a string, so no `includes` or `toString` of the caller's runs between the checks and the name
			if (typeof prefix !== 'string') throw new TypeError('fs.temp_dir takes a string prefix');
			assert_path_string(prefix);
			if (prefix.includes('/')) throw new TypeError(`fs.temp_dir takes a name prefix: ${prefix}`);
			// concatenated, not joined, so a prefix of `..` names `..XXXXXX`, never the parent
			const template = tmpdir() + '/' + prefix;
			try {
				const real_path = await realpath_text(await mkdtemp(template));
				return Object.freeze(new NodeFsTempDir(HANDLE_KEY, real_path));
			} catch (error) {
				throw to_fs_error(error, template);
			}
		}
	})
);
