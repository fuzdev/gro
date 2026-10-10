import type * as esbuild from 'esbuild';

import {
	render_sveltekit_shim_app_env,
	render_sveltekit_shim_app_paths,
	sveltekit_shim_app_specifiers
} from './sveltekit_shim_app.ts';
import type { ParsedSvelteConfig } from './svelte_config.ts';
import { EVERYTHING_MATCHER } from './constants.ts';

export interface EsbuildPluginSveltekitShimAppOptions {
	dev: boolean;
	base_url: ParsedSvelteConfig['base_url'];
	assets_url: ParsedSvelteConfig['assets_url'];
	version_name?: ParsedSvelteConfig['version_name'];
}

export const esbuild_plugin_sveltekit_shim_app = ({
	dev,
	base_url,
	assets_url,
	version_name
}: EsbuildPluginSveltekitShimAppOptions): esbuild.Plugin => ({
	name: 'sveltekit_shim_app',
	setup: (build) => {
		build.onResolve({ filter: /^\$app\/(forms|navigation|state)$/ }, ({ path, ...rest }) =>
			build.resolve(sveltekit_shim_app_specifiers.get(path)!, rest)
		);
		build.onResolve({ filter: /^\$app\/paths$/ }, ({ path }) => ({
			path: sveltekit_shim_app_specifiers.get(path)!,
			namespace: 'sveltekit_shim_app_paths'
		}));
		build.onLoad({ filter: EVERYTHING_MATCHER, namespace: 'sveltekit_shim_app_paths' }, () => ({
			contents: render_sveltekit_shim_app_paths(base_url, assets_url)
		}));
		build.onResolve({ filter: /^\$app\/env$/ }, ({ path }) => ({
			path: sveltekit_shim_app_specifiers.get(path)!,
			namespace: 'sveltekit_shim_app_env'
		}));
		build.onLoad({ filter: EVERYTHING_MATCHER, namespace: 'sveltekit_shim_app_env' }, () => ({
			contents: render_sveltekit_shim_app_env(dev, version_name)
		}));
	}
});
