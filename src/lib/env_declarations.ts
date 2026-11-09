/**
 * The shapes of a project's env var declarations and of loading the env,
 * apart from `env.ts`, which reads the env files, so the loader's hooks,
 * which need only these, never load the fs host module.
 *
 * @module
 */

import type { EnvVarConfig } from '@sveltejs/kit/env';

export interface LoadEnvOptions {
	/**
	 * Picks the `development` or `production` mode files for the default `env_files`.
	 */
	dev: boolean;
	/**
	 * The directory the env files are resolved against, absolute or relative to the project
	 * of the `FsPaths` that reads them, defaults to the project.
	 */
	env_dir?: string;
	/**
	 * The env files to load, in order, defaults to the same files as Vite.
	 */
	env_files?: Array<string>;
	/**
	 * The env merged in last, a plain record a root builds, like `process.env.get_all()`.
	 */
	ambient_env: Readonly<Record<string, string | undefined>>;
}

/**
 * The `variables` a project's `src/env.ts` exports via SvelteKit's `defineEnvVars`.
 * @see https://svelte.dev/docs/kit/environment-variables
 */
export type EnvVarDeclarations = Record<string, EnvVarConfig<any>>;

export type EnvVisibility = 'public' | 'private';

/**
 * Whether a declared env var belongs to `visibility` - SvelteKit's vars are private
 * unless declared `public: true`.
 */
export const env_var_has_visibility = (
	config: EnvVarConfig<any>,
	visibility: EnvVisibility
): boolean => (visibility === 'public') === !!config.public;
