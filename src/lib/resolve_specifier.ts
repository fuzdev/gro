import { stat } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';
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
 * @throws if the specifier doesn't name an existing file, with the fix when there is one
 *   and `code` `'ERR_MODULE_NOT_FOUND'` as Node sets it; any other filesystem error
 *   (like `EACCES`) is rethrown naming the specifier and importer, the original as `cause`
 */
export const resolve_specifier = async (
	specifier: string,
	importer: PathId
): Promise<ResolvedSpecifier> => {
	const raw = specifier.endsWith('?raw'); // TODO more robust detection? other values?
	const final_specifier = raw ? specifier.slice(0, -4) : specifier;
	const path_id = to_specifier_path_id(specifier, importer);

	let refusal;
	try {
		// Node reads a specifier as a URL, where a leading '//' names a host
		refusal = final_specifier.startsWith('//')
			? "a leading '//' names a URL host, not a path; write a single '/'"
			: await to_refusal(path_id, final_specifier);
	} catch (error) {
		throw new Error(`cannot resolve '${specifier}' from ${importer}: ${to_error_message(error)}`, {
			cause: error
		});
	}
	if (refusal !== null) {
		throw Object.assign(new Error(`cannot resolve '${specifier}' from ${importer}: ${refusal}`), {
			code: 'ERR_MODULE_NOT_FOUND'
		});
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
 * Why `path_id` doesn't load, or `null` when it names a file.
 * The sibling checks run only here, after the exact path fails,
 * to name the fix - resolution itself never probes.
 */
const to_refusal = async (path_id: string, specifier: string): Promise<string | null> => {
	const stats = await stat_or_null(path_id);
	if (stats?.isFile()) return null;
	const reason = stats
		? 'it is not a regular file (a directory or a special file); name a module file'
		: 'no such file';
	const sibling = await find_sibling(path_id, specifier);
	if (sibling !== null) {
		const why = extname(path_id) === '.js' ? '.js never maps to .ts' : 'nothing is probed';
		return `${reason}, but '${sibling}' exists; write the real extension (${why})`;
	}
	if (!stats && !extname(path_id)) {
		return `${reason}; the path has no extension and nothing is probed, so write the real one`;
	}
	return reason;
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
