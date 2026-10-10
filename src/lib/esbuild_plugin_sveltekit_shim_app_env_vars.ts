import type * as esbuild from 'esbuild';

import {
	load_sveltekit_env_declarations,
	render_sveltekit_shim_app_env_vars,
	resolve_sveltekit_env_entry,
	SVELTEKIT_APP_ENV_VARS_MATCHER
} from './sveltekit_shim_app_env_vars.ts';
import { EVERYTHING_MATCHER } from './constants.ts';
import type { EnvVisibility } from './env.ts';

export interface EsbuildPluginSveltekitShimAppEnvVarsOptions {
	dev: boolean;
	dir?: string;
	/**
	 * The SvelteKit `files.src`, where the `env.ts` declarations live.
	 */
	src_path?: string;
	env_dir?: string;
	env_files?: Array<string>;
	ambient_env?: Record<string, string>;
}

const namespace = 'sveltekit_shim_app_env_vars';

export const esbuild_plugin_sveltekit_shim_app_env_vars = ({
	dev,
	dir = process.cwd(),
	src_path = 'src',
	env_dir,
	env_files,
	ambient_env
}: EsbuildPluginSveltekitShimAppEnvVarsOptions): esbuild.Plugin => ({
	name: 'sveltekit_shim_app_env_vars',
	setup: (build) => {
		build.onResolve({ filter: SVELTEKIT_APP_ENV_VARS_MATCHER }, ({ path }) => ({
			path,
			namespace
		}));
		build.onLoad({ filter: EVERYTHING_MATCHER, namespace }, async ({ path }) => {
			const visibility = SVELTEKIT_APP_ENV_VARS_MATCHER.exec(path)![1] as EnvVisibility;
			const entry_id = resolve_sveltekit_env_entry(src_path, dir);
			// fresh so a rebuild after editing the declarations sees their new names
			const variables = entry_id ? await load_sveltekit_env_declarations(entry_id, true) : null;
			return {
				loader: 'js',
				resolveDir: dir,
				watchFiles: entry_id ? [entry_id] : undefined,
				contents: render_sveltekit_shim_app_env_vars({
					visibility,
					variables,
					entry_specifier: entry_id,
					dev,
					env_dir,
					env_files,
					ambient_env
				})
			};
		});
	}
});
