// on `node:fs`: `resolve_sveltekit_env_entry` runs in the loader's hooks, Node-only machinery
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import {
	env_var_has_visibility,
	type EnvVarDeclarations,
	type EnvVisibility,
	type LoadEnvOptions
} from './env_declarations.ts';

/**
 * Matches the SvelteKit env var modules, capturing the visibility.
 * @see https://svelte.dev/docs/kit/environment-variables
 */
export const SVELTEKIT_APP_ENV_VARS_MATCHER = /^\$app\/env\/(public|private)$/;

/**
 * The candidates for the env declarations module, in SvelteKit's order -
 * its default `moduleExtensions` are `.js` then `.ts`, and a directory index also counts.
 */
const SVELTEKIT_ENV_ENTRY_FILENAMES = ['env.js', 'env.ts', 'env/index.js', 'env/index.ts'];

/**
 * The project's env var declarations module, like `src/env.ts`, if it has one.
 * SvelteKit has no env vars without one, so neither does the shim.
 * @param src_path - the SvelteKit `files.src`, relative to the cwd
 */
export const resolve_sveltekit_env_entry = (src_path: string): string | null => {
	for (const filename of SVELTEKIT_ENV_ENTRY_FILENAMES) {
		const id = join(process.cwd(), src_path, filename);
		if (existsSync(id)) return id;
	}
	return null;
};

/**
 * Matches the names SvelteKit accepts for env vars, which become export names.
 */
const ENV_VAR_NAME_MATCHER = /^[a-zA-Z_$][a-zA-Z0-9_$]*$/;

/**
 * Imports the `variables` declared in a project's env entry module.
 * The shims render static exports, so the declared names are needed before rendering -
 * the values aren't, those resolve when the rendered module runs.
 * @param entry_id - the env declarations module, from `resolve_sveltekit_env_entry`
 * @throws if the entry doesn't export a `variables` object, or declares an invalid name
 */
export const load_sveltekit_env_declarations = async (
	entry_id: string
): Promise<EnvVarDeclarations> => {
	const { variables } = await import(pathToFileURL(entry_id).href);
	if (!variables || typeof variables !== 'object') {
		throw Error(`Expected ${entry_id} to export a \`variables\` object from \`defineEnvVars\``);
	}
	for (const name of Object.keys(variables)) {
		if (!ENV_VAR_NAME_MATCHER.test(name)) {
			throw Error(`Invalid env var name ${JSON.stringify(name)} in ${entry_id}`);
		}
	}
	return variables;
};

/**
 * A project's env declarations module and the vars it declares.
 */
export interface SveltekitEnvEntry {
	/**
	 * The module's path, from `resolve_sveltekit_env_entry`.
	 */
	id: string;
	/**
	 * The declarations from `load_sveltekit_env_declarations`.
	 */
	variables: EnvVarDeclarations;
}

/**
 * `dev` and `env_dir` are rendered into the module, which loads the env when it runs.
 */
export interface RenderSveltekitShimAppEnvVarsOptions extends Pick<
	LoadEnvOptions,
	'dev' | 'env_dir'
> {
	visibility: EnvVisibility;
	/**
	 * The project's env declarations module, `null` if it has none.
	 */
	entry: SveltekitEnvEntry | null;
	/**
	 * How the rendered module imports Gro's `env` module, a file URL.
	 */
	env_module_specifier: string;
	/**
	 * How the rendered module imports Gro's fs host module, a file URL.
	 */
	fs_module_specifier: string;
	/**
	 * How the rendered module imports Gro's `fs_paths` module, a file URL.
	 */
	fs_paths_module_specifier: string;
}

/**
 * Generates a module shim for SvelteKit's `$app/env/public` and `$app/env/private`.
 * Values resolve when the module runs, validated against the declarations,
 * so unlike SvelteKit's build, static vars aren't inlined - the module reads them
 * from the env files and ambient env, importing the declarations module and Gro's `env` module.
 * The rendered module is the composition root of that read: it imports the fs host module
 * and builds the `FsPaths` the env files are read through, so `$app/env/*` shows its fs reach
 * in its imports, while the shim's rendering, in the loader's hooks, never loads the host module.
 */
export const render_sveltekit_shim_app_env_vars = ({
	visibility,
	entry,
	env_module_specifier,
	fs_module_specifier,
	fs_paths_module_specifier,
	dev,
	env_dir
}: RenderSveltekitShimAppEnvVarsOptions): string => {
	const header = `// shim for $app/env/${visibility}
// @see https://svelte.dev/docs/kit/environment-variables
`;
	if (!entry) return header + 'export {};\n';
	const names = Object.entries(entry.variables)
		.filter(([, config]) => env_var_has_visibility(config, visibility))
		.map(([name]) => name);
	// an absent `env_dir` drops out, so `load_env` applies its default when the module runs
	const load_env_options = JSON.stringify({ dev, env_dir });
	// the module's own bindings are prefixed so no declared var name can collide with them
	return `${header}
import {variables as __gro_variables} from ${JSON.stringify(pathToFileURL(entry.id).href)};
import {
	load_env as __gro_load_env,
	resolve_env_vars as __gro_resolve_env_vars
} from ${JSON.stringify(env_module_specifier)};
import {fs as __gro_fs} from ${JSON.stringify(fs_module_specifier)};
import {create_fs_paths as __gro_create_fs_paths} from ${JSON.stringify(fs_paths_module_specifier)};

const __gro_env = __gro_resolve_env_vars(
	__gro_variables,
	${JSON.stringify(visibility)},
	await __gro_load_env(__gro_create_fs_paths(__gro_fs), ${load_env_options})
);

${names.map((name) => `export const ${name} = __gro_env.${name};`).join('\n')}
`;
};
