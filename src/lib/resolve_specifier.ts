import {extname, isAbsolute, join, relative} from 'node:path';
import {fs_exists} from '@fuzdev/fuz_util/fs.ts';
import type {PathId} from '@fuzdev/fuz_util/path.ts';

import {replace_extension} from './paths.ts';

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
	mapped_specifier: string;
	namespace: undefined | 'sveltekit_local_imports_ts' | 'sveltekit_local_imports_js';
	raw: boolean;
}

/**
 * Maps an import `specifier` relative to `dir`,
 * and infer the correct extension following Vite conventions.
 * If no `.js` file is found for the specifier on the filesystem, it assumes `.ts`.
 */
export const resolve_specifier = async (
	specifier: string,
	dir: string,
): Promise<ResolvedSpecifier> => {
	const raw = specifier.endsWith('?raw'); // TODO more robust detection? other values?
	const final_specifier = raw ? specifier.substring(0, specifier.length - 4) : specifier;
	const absolute_path = isAbsolute(final_specifier) ? final_specifier : join(dir, final_specifier);

	let mapped_path;
	let path_id;
	let namespace: ResolvedSpecifier['namespace'];

	const ext = extname(absolute_path);
	const is_js = ext === '.js';
	const is_ts = ext === '.ts';

	if (!is_js && !is_ts && (await fs_exists(absolute_path))) {
		// unrecognized extension and the file exists
		mapped_path = absolute_path;
		path_id = absolute_path;
	} else if (is_ts) {
		// explicitly ts
		mapped_path = replace_extension(absolute_path, '.js');
		path_id = absolute_path;
		namespace = 'sveltekit_local_imports_ts';
	} else {
		// extensionless, or js that points to ts, or just js
		const js_id = is_js ? absolute_path : absolute_path + '.js';
		const ts_id = is_js ? replace_extension(absolute_path, '.ts') : absolute_path + '.ts';
		const [ts_exists, js_exists] = await Promise.all([fs_exists(ts_id), fs_exists(js_id)]);
		if (!ts_exists && js_exists) {
			mapped_path = js_id;
			path_id = js_id;
			namespace = 'sveltekit_local_imports_js';
		} else {
			mapped_path = js_id;
			path_id = ts_id;
			namespace = 'sveltekit_local_imports_ts';
		}
	}

	let mapped_specifier = relative(dir, mapped_path);
	if (mapped_specifier[0] !== '.') mapped_specifier = './' + mapped_specifier;

	return {
		path_id,
		path_id_with_querystring: raw ? path_id + '?raw' : path_id,
		raw,
		specifier,
		mapped_specifier,
		namespace,
	};
};
