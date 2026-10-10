import type { EnvVarConfig } from '@sveltejs/kit/env';
import dotenv from 'dotenv';
import { resolve } from 'node:path';
import { existsSync, readFileSync } from 'node:fs';

/**
 * Loads every env var from the env files plus the ambient env, later sources winning,
 * so the ambient env overrides the files.
 * @param dev - picks the `development` or `production` mode files for the default `env_files`
 * @param env_dir - the directory the env files are resolved against, defaults to the cwd
 * @param env_files - the env files to load, in order, defaults to the same files as Vite
 * @param ambient_env - the env merged in last
 * @returns all loaded vars, unfiltered - `resolve_env_vars` picks out the declared ones
 */
export const load_env = (
	dev: boolean,
	env_dir?: string,
	env_files = to_default_env_files(dev),
	ambient_env: Record<string, string | undefined> = process.env
): Record<string, string> => {
	const env: Record<string, string> = {};
	for (const path of env_files) {
		const loaded = load(env_dir === undefined ? path : resolve(env_dir, path));
		if (loaded) Object.assign(env, loaded);
	}
	for (const key in ambient_env) {
		const value = ambient_env[key];
		if (value !== undefined) env[key] = value;
	}
	return env;
};

/**
 * The env files Vite loads for a mode, in precedence order, later files winning.
 * @see https://vite.dev/guide/env-and-mode.html#env-files
 */
export const to_default_env_files = (dev: boolean): Array<string> => {
	const mode = dev ? 'development' : 'production';
	return ['.env', '.env.local', `.env.${mode}`, `.env.${mode}.local`];
};

/**
 * Loads a single env value without merging it into `process.env`.
 * By default searches process.env, then a local `.env` if one exists, then `../.env` if it exists.
 * Empty strings are semantically the same as `undefined` for more ergonomic fallbacks.
 */
export const load_from_env = (key: string, paths = ['.env', '../.env']): string | undefined => {
	let v = process.env[key];
	if (v) return v;
	for (const path of paths) {
		const env = load(path);
		v = env?.[key];
		if (v) return v;
	}
	return undefined;
};

const load = (path: string): Record<string, string> | undefined => {
	if (!existsSync(path)) return;
	const loaded = readFileSync(path, 'utf8');
	return dotenv.parse(loaded);
};

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

type EnvVarSchema = NonNullable<EnvVarConfig<any>['schema']>;
type StandardSchema = Exclude<EnvVarSchema, (value: string | undefined) => any>;

/**
 * Normalizes a function validator to a Standard Schema, like `defineEnvVars` does,
 * for declarations that didn't go through it. Standard Schemas can be callable (ArkType),
 * so a function is only a validator if it isn't one.
 */
const to_standard_schema = (schema: EnvVarSchema | undefined): StandardSchema | undefined => {
	if (schema === undefined || '~standard' in schema) return schema;
	return {
		'~standard': {
			version: 1,
			vendor: 'gro',
			validate: (value) => {
				try {
					const result = schema(value as string | undefined);
					// handed back so `resolve_env_vars` reports it as an unsupported async validator
					if (result instanceof Promise) return result;
					return { value: result };
				} catch (error) {
					return { issues: [{ message: error instanceof Error ? error.message : String(error) }] };
				}
			}
		}
	};
};

/**
 * Resolves the declared env vars of one visibility from `env`, validating each the way
 * SvelteKit does: a var with no `schema` must be present, and one with a `schema` gets its
 * Standard Schema result, so validators can transform values and make them optional.
 * @param variables - the declarations from the project's `src/env.ts`
 * @param visibility - which of the declared vars to resolve
 * @param env - the values to resolve them from, usually from `load_env`
 * @returns the resolved values keyed by var name
 * @throws listing every invalid var, so a misconfigured env fails all at once
 */
export const resolve_env_vars = (
	variables: EnvVarDeclarations,
	visibility: EnvVisibility,
	env: Record<string, string | undefined>
): Record<string, unknown> => {
	const resolved: Record<string, unknown> = {};
	const issues: Array<string> = [];

	for (const [name, config] of Object.entries(variables)) {
		if (!env_var_has_visibility(config, visibility)) continue;
		const value = env[name];
		const validator = to_standard_schema(config.schema);
		if (!validator) {
			if (value === undefined) {
				issues.push(
					`${name}: value is missing - declare a schema that accepts it to make it optional`
				);
			}
			resolved[name] = value;
			continue;
		}
		const result = validator['~standard'].validate(value);
		if (result instanceof Promise) {
			issues.push(`${name}: async validators are not supported`);
			continue;
		}
		if (result.issues) {
			issues.push(`${name}: ${result.issues.map((i) => i.message).join('; ')}`);
			continue;
		}
		resolved[name] = result.value;
	}

	if (issues.length) {
		throw Error(`Invalid ${visibility} env vars:\n${issues.map((i) => '- ' + i).join('\n')}`);
	}

	return resolved;
};
