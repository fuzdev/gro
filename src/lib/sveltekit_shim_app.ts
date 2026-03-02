import type {ParsedSvelteConfig} from './svelte_config.ts';

export const SVELTEKIT_SHIM_APP_PATHS_MATCHER = /\/util\/sveltekit_shim_app_paths\.js$/;
export const SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER = /\/util\/sveltekit_shim_app_environment\.js$/;

/**
 * Maps SvelteKit `$app` specifiers to their Gro shims.
 * @see https://kit.svelte.dev/docs/modules
 */
export const sveltekit_shim_app_specifiers = new Map([
	['$app/environment', '@fuzdev/gro/sveltekit_shim_app_environment.js'],
	['$app/forms', '@fuzdev/gro/sveltekit_shim_app_forms.js'],
	['$app/navigation', '@fuzdev/gro/sveltekit_shim_app_navigation.js'],
	['$app/paths', '@fuzdev/gro/sveltekit_shim_app_paths.js'],
	['$app/state', '@fuzdev/gro/sveltekit_shim_app_state.js'],
]);

export const render_sveltekit_shim_app_paths = (
	base_url: ParsedSvelteConfig['base_url'] = '',
	assets_url: ParsedSvelteConfig['assets_url'] = '',
): string => `// shim for $app/paths
// @see https://github.com/sveltejs/kit/issues/1485

export const assets = ${JSON.stringify(assets_url)};
export const base = ${JSON.stringify(base_url)};
`;

// TODO improve support
// `dev` is not guaranteed to be the same as `MODE` - https://svelte.dev/docs/kit/$app-environment
// `version` is `config.kit.version.name` but I couldn't see how to load a SvelteKit `ValidatedConfig`
// `building` is just being hardcoded, might be better (but still not correct) to be `!dev`
export const render_sveltekit_shim_app_environment = (
	dev: boolean,
): string => `// shim for $app/environment
// @see https://github.com/sveltejs/kit/issues/1485

export const browser = false;
export const building = false;
export const dev = ${JSON.stringify(dev)};
export const version = 'TODO';
`;
