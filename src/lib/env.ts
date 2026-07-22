import dotenv from 'dotenv';
import { resolve } from 'node:path';
import { existsSync, readFileSync } from 'node:fs';

export const load_env = (
	dev: boolean,
	visibility: 'public' | 'private',
	public_prefix: string,
	private_prefix: string,
	env_dir?: string,
	env_files = ['.env', '.env.' + (dev ? 'development' : 'production')],
	ambient_env = process.env
): Record<string, string> => {
	const envs: Array<Record<string, string | undefined>> = env_files
		.map((path) => load(env_dir === undefined ? path : resolve(env_dir, path)))
		.filter((v) => v !== undefined);
	envs.push(ambient_env);
	return merge_envs(envs, visibility, public_prefix, private_prefix);
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

export const merge_envs = (
	envs: Array<Record<string, string | undefined>>,
	visibility: 'public' | 'private',
	public_prefix: string,
	private_prefix: string
): Record<string, string> => {
	const env: Record<string, string> = {};

	for (const e of envs) {
		for (const key in e) {
			if (
				(visibility === 'private' && is_private_env(key, public_prefix, private_prefix)) ||
				(visibility === 'public' && is_public_env(key, public_prefix, private_prefix))
			) {
				const value = e[key];
				if (value !== undefined) env[key] = value;
			}
		}
	}

	return env;
};

export const is_private_env = (
	key: string,
	public_prefix: string,
	private_prefix: string
): boolean =>
	key.startsWith(private_prefix) && (public_prefix === '' || !key.startsWith(public_prefix));

export const is_public_env = (
	key: string,
	public_prefix: string,
	private_prefix: string
): boolean =>
	key.startsWith(public_prefix) && (private_prefix === '' || !key.startsWith(private_prefix));
