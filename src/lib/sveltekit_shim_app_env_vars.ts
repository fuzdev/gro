import { existsSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { env_var_has_visibility, type EnvVarDeclarations, type EnvVisibility } from './env.ts';

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
 * @param src_path - the SvelteKit `files.src`, relative to `dir`
 * @param dir - the project directory
 */
export const resolve_sveltekit_env_entry = (
	src_path: string,
	dir = process.cwd()
): string | null => {
	for (const filename of SVELTEKIT_ENV_ENTRY_FILENAMES) {
		const id = join(dir, src_path, filename);
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
 * @param entry_id - the env entry module
 * @param fresh - re-import the entry when it has changed on disk, for long-lived processes
 * that rebuild, at the cost of a new module instance per change
 * @throws if the entry doesn't export a `variables` object, or declares an invalid name
 */
export const load_sveltekit_env_declarations = async (
	entry_id: string,
	fresh = false
): Promise<EnvVarDeclarations> => {
	const url = pathToFileURL(entry_id);
	if (fresh) url.searchParams.set('t', String(statSync(entry_id).mtimeMs));
	const { variables } = await import(url.href);
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

export interface RenderSveltekitShimAppEnvVarsOptions {
	visibility: EnvVisibility;
	/**
	 * The declarations from `load_sveltekit_env_declarations`, `null` if the project has none.
	 */
	variables: EnvVarDeclarations | null;
	/**
	 * How the rendered module imports the env entry - a file URL for Node, a path for esbuild.
	 */
	entry_specifier: string | null;
	/**
	 * How the rendered module imports Gro's `env` module.
	 * @default `'@fuzdev/gro/env.js'`
	 */
	env_module_specifier?: string;
	dev: boolean;
	env_dir?: string;
	env_files?: Array<string>;
	ambient_env?: Record<string, string | undefined>;
}

/**
 * Generates a module shim for SvelteKit's `$app/env/public` and `$app/env/private`.
 * Values resolve when the module runs, validated against the declarations,
 * so unlike SvelteKit's build, static vars aren't inlined - a bundled server reads them
 * from its env files and ambient env at startup, and imports the declarations module
 * and Gro's `env` module at runtime.
 */
export const render_sveltekit_shim_app_env_vars = ({
	visibility,
	variables,
	entry_specifier,
	env_module_specifier = '@fuzdev/gro/env.js',
	dev,
	env_dir,
	env_files,
	ambient_env
}: RenderSveltekitShimAppEnvVarsOptions): string => {
	const header = `// shim for $app/env/${visibility}
// @see https://svelte.dev/docs/kit/environment-variables
`;
	if (!variables || !entry_specifier) return header + 'export {};\n';
	const names = Object.entries(variables)
		.filter(([, config]) => env_var_has_visibility(config, visibility))
		.map(([name]) => name);
	// `JSON.stringify(undefined)` is `undefined`, not a string, so absent args are spelled out
	const load_env_args = [dev, env_dir, env_files, ambient_env]
		.map((arg) => (arg === undefined ? 'undefined' : JSON.stringify(arg)))
		.join(', ');
	// the module's own bindings are prefixed so no declared var name can collide with them
	return `${header}
import {variables as __gro_variables} from ${JSON.stringify(entry_specifier)};
import {
	load_env as __gro_load_env,
	resolve_env_vars as __gro_resolve_env_vars
} from ${JSON.stringify(env_module_specifier)};

const __gro_env = __gro_resolve_env_vars(
	__gro_variables,
	${JSON.stringify(visibility)},
	__gro_load_env(${load_env_args})
);

${names.map((name) => `export const ${name} = __gro_env.${name};`).join('\n')}
`;
};
