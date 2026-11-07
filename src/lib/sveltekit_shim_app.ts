import type { ParsedSvelteConfig } from './svelte_config.ts';

/**
 * Maps SvelteKit `$app` specifiers to the names of their Gro shim modules,
 * which sit beside the loader.
 * `$app/env/public` and `$app/env/private` aren't files, see `sveltekit_shim_app_env_vars.ts`.
 * @see https://svelte.dev/docs/kit/$app-env
 */
export const SVELTEKIT_SHIM_APP_SPECIFIERS = new Map([
	['$app/env', 'sveltekit_shim_app_env'],
	['$app/forms', 'sveltekit_shim_app_forms'],
	['$app/navigation', 'sveltekit_shim_app_navigation'],
	['$app/paths', 'sveltekit_shim_app_paths'],
	['$app/state', 'sveltekit_shim_app_state']
]);

// TODO route ids with params aren't filled in, only paths are prefixed
export const render_sveltekit_shim_app_paths = (
	base_url: ParsedSvelteConfig['base_url'] = '',
	assets_url: ParsedSvelteConfig['assets_url'] = ''
): string => `// shim for $app/paths
// @see https://github.com/sveltejs/kit/issues/1485

const base = ${JSON.stringify(base_url)};
const assets = ${JSON.stringify(assets_url || base_url)};

export const resolve = (v) => base + '/' + v.replace(/^\\//, '');
export const asset = (v) => assets + '/' + v.replace(/^\\//, '');
export const match = () => Promise.resolve(null);
`;

// TODO `dev` is not guaranteed to be the same as `MODE` - https://svelte.dev/docs/kit/$app-env
// `building` is hardcoded, might be better (but still not correct) to be `!dev`
export const render_sveltekit_shim_app_env = (
	dev: boolean,
	version_name: ParsedSvelteConfig['version_name'] = ''
): string => `// shim for $app/env
// @see https://github.com/sveltejs/kit/issues/1485

export const browser = false;
export const building = false;
export const dev = ${JSON.stringify(dev)};
export const version = ${JSON.stringify(version_name)};
`;
