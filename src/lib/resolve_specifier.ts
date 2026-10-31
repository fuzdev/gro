import { stat } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import type { PathId } from '@fuzdev/fuz_util/path.ts';
import { to_error_message } from '@fuzdev/fuz_util/error.ts';

import { replace_extension } from './paths.ts';

/**
 * A relative or absolute specifier resolved by `resolve_specifier`.
 */
export interface ResolvedSpecifier {
	/**
	 * The resolved filesystem path for the specifier.
	 */
	path_id: PathId;
	specifier: string;
	namespace: undefined | 'sveltekit_local_imports_ts' | 'sveltekit_local_imports_js';
	/**
	 * Whether the specifier ends in `?raw`, importing the file's text.
	 */
	raw: boolean;
}

/**
 * The query that imports a file's text as its default export, the only query gro recognizes.
 */
export const RAW_QUERY = '?raw';

/**
 * Strips a trailing `?raw` from `specifier`.
 * @param specifier - an import specifier or URL, optionally ending in `?raw`
 */
export const strip_raw_query = (specifier: string): string =>
	specifier.endsWith(RAW_QUERY) ? specifier.slice(0, -RAW_QUERY.length) : specifier;

const DIRECTORY_LIKE_MATCHER = /(^|\/)\.{0,2}$/;

/**
 * Matches a path specifier, the rule `is_path_specifier` and esbuild filters share.
 */
export const PATH_SPECIFIER_MATCHER = /^(\.\.?(\/|$)|\/)/;

/**
 * Whether an import `specifier` is a path, relative or absolute,
 * as Node reads one: `./`, `../`, or `/` starting it, or exactly `.` or `..`.
 * Anything else, like `'.hidden.ts'`, is bare and resolves as a package.
 * @param specifier - an import specifier
 */
export const is_path_specifier = (specifier: string): boolean =>
	PATH_SPECIFIER_MATCHER.test(specifier);

/**
 * The path a relative or absolute `specifier` names, whether or not it exists.
 * @param specifier - a relative or absolute path, optionally ending in `?raw`
 * @param importer - the path of the importing module
 */
export const to_specifier_path_id = (specifier: string, importer: PathId): PathId => {
	const final_specifier = strip_raw_query(specifier);
	const path_id = resolve(dirname(importer), final_specifier);
	// a last segment of '', '.', or '..' names a directory, so keep the slash
	// and a file is never reached through one, as in Node
	return DIRECTORY_LIKE_MATCHER.test(final_specifier) && path_id !== '/' ? path_id + '/' : path_id;
};

/**
 * Resolves a relative or absolute import `specifier` to exactly the file it names,
 * the rule gro's runtime resolver follows: extensions are mandatory,
 * nothing is probed, and `.js` never maps to `.ts`.
 * The specifier is read as a filesystem path, never URL-decoded; `?raw` is the only query recognized.
 * @param specifier - a relative or absolute path, optionally ending in `?raw`
 * @param importer - the path of the importing module
 * @throws if the specifier doesn't name an existing file, with the fix when there is one,
 *   `code` `'ERR_UNSUPPORTED_DIR_IMPORT'` for a directory or else `'ERR_MODULE_NOT_FOUND'`,
 *   and the path's `url` (with any `?raw`), as Node sets them; any other filesystem error
 *   (like `EACCES`) is rethrown naming the specifier and importer, the original as `cause`
 */
export const resolve_specifier = async (
	specifier: string,
	importer: PathId
): Promise<ResolvedSpecifier> => {
	const final_specifier = strip_raw_query(specifier);
	const raw = final_specifier !== specifier;
	const path_id = to_specifier_path_id(specifier, importer);

	let refusal: Refusal | null;
	try {
		// Node reads a specifier as a URL, where a leading '//' names a host
		refusal = final_specifier.startsWith('//')
			? {
					reason: "a leading '//' names a URL host, not a path; write a single '/'",
					code: 'ERR_MODULE_NOT_FOUND'
				}
			: await to_refusal(path_id, specifier);
	} catch (error) {
		throw new Error(`cannot resolve '${specifier}' from ${importer}: ${to_error_message(error)}`, {
			cause: error
		});
	}
	if (refusal !== null) {
		// `code` and `url` as Node sets them, so `import.meta.resolve` returns the URL
		// for a path that doesn't load, as it does without the loader
		throw Object.assign(
			new Error(`cannot resolve '${specifier}' from ${importer}: ${refusal.reason}`),
			{ code: refusal.code, url: pathToFileURL(path_id).href + (raw ? RAW_QUERY : '') }
		);
	}

	const ext = extname(path_id);
	const namespace: ResolvedSpecifier['namespace'] =
		ext === '.ts'
			? 'sveltekit_local_imports_ts'
			: ext === '.js'
				? 'sveltekit_local_imports_js'
				: undefined;

	return {
		path_id,
		raw,
		specifier,
		namespace
	};
};

/**
 * Why a path doesn't load, and the error `code` Node gives it.
 */
interface Refusal {
	reason: string;
	code: 'ERR_MODULE_NOT_FOUND' | 'ERR_UNSUPPORTED_DIR_IMPORT';
}

/**
 * Why `path_id` doesn't load, or `null` when it names a file.
 * The sibling checks run only after the exact path fails,
 * to name the fix - resolution itself never probes.
 * @param specifier - the specifier as written, optionally ending in `?raw`, which the named fix keeps
 */
const to_refusal = async (path_id: string, specifier: string): Promise<Refusal | null> => {
	const stats = await stat_or_null(path_id);
	if (stats?.isFile()) return null;
	const code = stats?.isDirectory() ? 'ERR_UNSUPPORTED_DIR_IMPORT' : 'ERR_MODULE_NOT_FOUND';
	const reason = stats
		? 'it is not a regular file (a directory or a special file); name a module file'
		: 'no such file';
	const sibling = await to_sibling_hint(path_id, specifier);
	if (sibling !== null) return { reason: `${reason}, but ${sibling.hint}`, code };
	if (!stats && !extname(path_id)) {
		return {
			reason: `${reason}; the path has no extension and nothing is probed, so write the real one`,
			code
		};
	}
	return { reason, code };
};

/**
 * The file a missing path likely meant, the specifier naming it, and the hint naming the fix.
 * @internal
 */
export interface SiblingHint {
	/**
	 * The existing file.
	 */
	sibling_id: PathId;
	/**
	 * The specifier as written with the sibling's extension, keeping any `?raw`.
	 */
	sibling_specifier: string;
	/**
	 * The fix, like `'./x.ts' exists; write the real extension (.js never maps to .ts)`.
	 */
	hint: string;
}

/**
 * Finds the file a missing `path_id` likely meant, when it exists:
 * `.ts` in place of `.js` or `.js` in place of `.ts`,
 * or for a path with no extension, `.ts` or `.js` appended.
 * Shared by the resolver's refusal and the loader's subpath imports,
 * so each names the fix the same way.
 * @param path_id - the missing path `specifier` names
 * @param specifier - the specifier as written, optionally ending in `?raw`, which the fix keeps
 * @returns the sibling and the hint, or `null` when no sibling exists
 * @internal
 */
export const to_sibling_hint = async (
	path_id: PathId,
	specifier: string
): Promise<SiblingHint | null> => {
	const bare_specifier = strip_raw_query(specifier);
	const query = specifier.slice(bare_specifier.length);
	const ext = extname(path_id);
	const swapped_ext = ext === '.js' ? '.ts' : ext === '.ts' ? '.js' : null;
	if (swapped_ext !== null) {
		const sibling_id = replace_extension(path_id, swapped_ext);
		if (!(await stat_or_null(sibling_id))?.isFile()) return null;
		const sibling_specifier = replace_extension(bare_specifier, swapped_ext) + query;
		return {
			sibling_id,
			sibling_specifier,
			hint: `'${sibling_specifier}' exists; write the real extension (${ext} never maps to ${swapped_ext})`
		};
	}
	if (ext) return null; // only a path with no extension is a sibling's stem
	for (const appended of ['.ts', '.js']) {
		const sibling_id = path_id + appended;
		if ((await stat_or_null(sibling_id))?.isFile()) {
			const sibling_specifier = bare_specifier + appended + query;
			return {
				sibling_id,
				sibling_specifier,
				hint: `'${sibling_specifier}' exists; write the real extension (nothing is probed)`
			};
		}
	}
	return null;
};

const stat_or_null = async (path: string): Promise<Awaited<ReturnType<typeof stat>> | null> => {
	try {
		return await stat(path);
	} catch (error) {
		const code = (error as NodeJS.ErrnoException).code;
		if (code === 'ENOENT' || code === 'ENOTDIR') return null;
		throw error;
	}
};
