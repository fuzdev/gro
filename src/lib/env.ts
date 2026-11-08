import type { EnvVarConfig } from '@sveltejs/kit/env';
import dotenv from 'dotenv';
import { resolve } from 'node:path';

import {
	env_var_has_visibility,
	type EnvVarDeclarations,
	type EnvVisibility,
	type LoadEnvOptions
} from './env_declarations.ts';
import { fs_open_path } from './internal/fs_path.ts';

/**
 * Loads every env var from the env files plus the ambient env, later sources winning,
 * so the ambient env overrides the files.
 * A missing env file is skipped; one that can't be read throws.
 * @returns all loaded vars, unfiltered - `resolve_env_vars` picks out the declared ones
 * @throws `FsError` - when reading an env file fails other than by its absence
 */
export const load_env = async ({
	dev,
	env_dir,
	env_files = to_default_env_files(dev),
	ambient_env = process.env
}: LoadEnvOptions): Promise<Record<string, string>> => {
	const env: Record<string, string> = {};
	const loaded_files = await Promise.all(
		env_files.map((path) => load(env_dir === undefined ? path : resolve(env_dir, path)))
	);
	for (const loaded of loaded_files) {
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
 * Searches `process.env`, then each of `paths` that exists, by default only the cwd's `.env` -
 * never a parent directory's, so one directory's `.env` isn't a secret channel to every repo under it.
 * Empty strings are semantically the same as `undefined` for more ergonomic fallbacks.
 * @throws `FsError` - when reading one of `paths` fails other than by its absence
 */
export const load_from_env = async (key: string, paths = ['.env']): Promise<string | undefined> => {
	let v = process.env[key];
	if (v) return v;
	for (const path of paths) {
		const env = await load(path);
		v = env?.[key];
		if (v) return v;
	}
	return undefined;
};

const load = async (path: string): Promise<Record<string, string> | undefined> => {
	const opened = await fs_open_path(path);
	const loaded = await opened.dir.read_text_if_exists(opened.path);
	return loaded === null ? undefined : dotenv.parse(loaded);
};

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
