// shim for $app/paths
// @see https://github.com/sveltejs/kit/issues/1485
// @see https://svelte.dev/docs/kit/$app-paths

/**
 * This file is created dynamically by `render_sveltekit_shim_app_paths`
 * but exists here for the sake of the Node loader.
 * There may be a cleaner workaround but I couldn't find it.
 * @see https://github.com/nodejs/loaders for details about the forthcoming virtual file support
 *
 * @module
 */

import type {
	resolve as base_resolve,
	asset as base_asset,
	resolveRoute as base_resolveRoute,
} from '$app/paths';
import {noop} from '@fuzdev/fuz_util/function.js';

export const assets = '';
/** @deprecated */
export const base = '';
export const resolve: typeof base_resolve = (v, ..._rest) => ('/' + v.replace(/^\//, '')) as any; // TODO needs to use SvelteKit config base
/** @deprecated */
export const resolveRoute: typeof base_resolveRoute = noop; // eslint-disable-line @typescript-eslint/no-deprecated
export const asset: typeof base_asset = (v) => ('/' + v.replace(/^\//, '')) as any; // TODO needs to use SvelteKit config base
