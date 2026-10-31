import { stat } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import type { PathId } from '@fuzdev/fuz_util/path.ts';
import { to_error_message } from '@fuzdev/fuz_util/error.ts';

import { replace_extension } from './paths.ts';

// TODO ideally this module doesnt exist, but import.meta.resolve doesn't work in loaders last I tried

export interface ResolvedSpecifier {
	/**
	 * The resolved filesystem path for the specifier.
	 */
	path_id: PathId;
	/**
	 * Same as `path_id` but includes `?raw` and other querystrings. (currently none)
	 */
	path_id_with_querystring: string;
	specifier: string;
	namespace: undefined | 'sveltekit_local_imports_ts' | 'sveltekit_local_imports_js';
	raw: boolean;
}

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
	const final_specifier = specifier.endsWith('?raw') ? specifier.slice(0, -4) : specifier;
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
	const raw = specifier.endsWith('?raw'); // TODO more robust detection? other values?
	const final_specifier = raw ? specifier.slice(0, -4) : specifier;
	const path_id = to_specifier_path_id(specifier, importer);

	let refusal: Refusal | null;
	try {
		// Node reads a specifier as a URL, where a leading '//' names a host
		refusal = final_specifier.startsWith('//')
			? {
					reason: "a leading '//' names a URL host, not a path; write a single '/'",
					code: 'ERR_MODULE_NOT_FOUND'
				}
			: await to_refusal(path_id, final_specifier, raw);
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
			{ code: refusal.code, url: pathToFileURL(path_id).href + (raw ? '?raw' : '') }
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
		path_id_with_querystring: raw ? path_id + '?raw' : path_id,
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
 * The sibling checks run only here, after the exact path fails,
 * to name the fix - resolution itself never probes.
 * @param raw - whether the specifier ends in `?raw`, which the named fix keeps
 */
const to_refusal = async (
	path_id: string,
	specifier: string,
	raw: boolean
): Promise<Refusal | null> => {
	const stats = await stat_or_null(path_id);
	if (stats?.isFile()) return null;
	const code = stats?.isDirectory() ? 'ERR_UNSUPPORTED_DIR_IMPORT' : 'ERR_MODULE_NOT_FOUND';
	const reason = stats
		? 'it is not a regular file (a directory or a special file); name a module file'
		: 'no such file';
	const sibling = await find_sibling(path_id, specifier);
	if (sibling !== null) {
		const why = extname(path_id) === '.js' ? '.js never maps to .ts' : 'nothing is probed';
		return {
			reason: `${reason}, but '${raw ? sibling + '?raw' : sibling}' exists; write the real extension (${why})`,
			code
		};
	}
	if (!stats && !extname(path_id)) {
		return {
			reason: `${reason}; the path has no extension and nothing is probed, so write the real one`,
			code
		};
	}
	return { reason, code };
};

/**
 * The specifier a missing `path_id` likely meant:
 * `.ts` in place of `.js`, or `.ts` or `.js` appended.
 */
const find_sibling = async (path_id: string, specifier: string): Promise<string | null> => {
	if (extname(path_id) === '.js') {
		const ts_id = replace_extension(path_id, '.ts');
		return (await stat_or_null(ts_id))?.isFile() ? replace_extension(specifier, '.ts') : null;
	}
	for (const ext of ['.ts', '.js']) {
		if ((await stat_or_null(path_id + ext))?.isFile()) return specifier + ext;
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
