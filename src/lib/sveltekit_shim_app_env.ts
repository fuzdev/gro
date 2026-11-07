// shim for $app/env
// @see https://github.com/sveltejs/kit/issues/1485
// @see https://svelte.dev/docs/kit/$app-env

/**
 * A placeholder: the loader resolves `$app/env` to this module
 * and loads it with the source `render_sveltekit_shim_app_env` renders from the Svelte config.
 * @see https://github.com/nodejs/loaders for details about the forthcoming virtual file support
 *
 * @module
 */

export const browser = false;
export const building = false;
export const dev = true;
export const version = '';
