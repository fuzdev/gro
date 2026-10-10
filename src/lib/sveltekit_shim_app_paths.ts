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

import type { resolve as base_resolve, asset as base_asset, match as base_match } from '$app/paths';

// TODO route ids with params aren't filled in, only paths are prefixed
export const resolve: typeof base_resolve = (v, ..._rest) => ('/' + v.replace(/^\//, '')) as any;
export const asset: typeof base_asset = (v) => '/' + v.replace(/^\//, '');
export const match: typeof base_match = () => Promise.resolve(null);
