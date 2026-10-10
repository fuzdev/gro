import type { ParsedSvelteConfig } from './svelte_config.ts';

// matched against the URL the loader resolves the shim specifier to, in `dist/` or `src/lib/`
export const SVELTEKIT_SHIM_APP_PATHS_MATCHER = /\/sveltekit_shim_app_paths\.(js|ts)$/;
export const SVELTEKIT_SHIM_APP_ENV_MATCHER = /\/sveltekit_shim_app_env\.(js|ts)$/;

/**
 * Maps SvelteKit `$app` specifiers to their Gro shims.
 * `$app/env/public` and `$app/env/private` aren't files, see `sveltekit_shim_app_env_vars.ts`.
 * @see https://svelte.dev/docs/kit/$app-env
 */
export const sveltekit_shim_app_specifiers = new Map([
	['$app/env', '@fuzdev/gro/sveltekit_shim_app_env.js'],
	['$app/forms', '@fuzdev/gro/sveltekit_shim_app_forms.js'],
	['$app/navigation', '@fuzdev/gro/sveltekit_shim_app_navigation.js'],
	['$app/paths', '@fuzdev/gro/sveltekit_shim_app_paths.js'],
	['$app/state', '@fuzdev/gro/sveltekit_shim_app_state.js']
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
