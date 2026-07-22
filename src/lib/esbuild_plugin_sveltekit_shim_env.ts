import type * as esbuild from 'esbuild';

import { render_env_shim_module } from './sveltekit_shim_env.ts';
import { SVELTEKIT_ENV_MATCHER, EVERYTHING_MATCHER } from './constants.ts';

export interface EsbuildPluginSveltekitShimEnvOptions {
	dev: boolean;
	public_prefix?: string;
	private_prefix?: string;
	env_dir?: string;
	env_files?: Array<string>;
	ambient_env?: Record<string, string>;
}

const namespace = 'sveltekit_shim_env';

export const esbuild_plugin_sveltekit_shim_env = ({
	dev,
	public_prefix,
	private_prefix,
	env_dir,
	env_files,
	ambient_env
}: EsbuildPluginSveltekitShimEnvOptions): esbuild.Plugin => ({
	name: 'sveltekit_shim_env',
	setup: (build) => {
		build.onResolve({ filter: SVELTEKIT_ENV_MATCHER }, ({ path }) => ({ path, namespace }));
		build.onLoad({ filter: EVERYTHING_MATCHER, namespace }, ({ path }) => {
			const matches = SVELTEKIT_ENV_MATCHER.exec(path);
			const mode = matches![1] as 'static' | 'dynamic';
			const visibility = matches![2] as 'public' | 'private';
			return {
				loader: 'ts',
				contents: render_env_shim_module(
					dev,
					mode,
					visibility,
					public_prefix,
					private_prefix,
					env_dir,
					env_files,
					ambient_env
				)
			};
		});
	}
});
