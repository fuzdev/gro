import {args_serialize} from '@fuzdev/fuz_util/args.js';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {PackageJson} from '@fuzdev/fuz_util/package_json.js';
import type {Result} from '@fuzdev/fuz_util/result.js';
import {join} from 'node:path';

import {to_forwarded_args} from './args.ts';
import {find_cli, spawn_cli, to_cli_name, type Cli} from './cli.ts';
import {
	PM_CLI_DEFAULT,
	SVELTE_CONFIG_FILENAME,
	SVELTE_PACKAGE_DEP_NAME,
	SVELTEKIT_CLI,
	SVELTEKIT_DEV_DIRNAME,
} from './constants.ts';
import {package_json_has_dependency} from './package_json.ts';
import {default_svelte_config, type ParsedSvelteConfig} from './svelte_config.ts';
import {TaskError} from './task.ts';

export const has_sveltekit_app = async (
	svelte_config_path: string = SVELTE_CONFIG_FILENAME,
): Promise<Result<object, {message: string}>> => {
	if (!(await fs_exists(svelte_config_path))) {
		return {ok: false, message: `no SvelteKit config found at ${SVELTE_CONFIG_FILENAME}`};
	}
	// TODO check for routes?
	return {ok: true};
};

export const has_sveltekit_library = async (
	package_json: PackageJson,
	svelte_config: ParsedSvelteConfig = default_svelte_config,
	dep_name = SVELTE_PACKAGE_DEP_NAME,
): Promise<Result<object, {message: string}>> => {
	const has_sveltekit_app_result = await has_sveltekit_app();
	if (!has_sveltekit_app_result.ok) {
		return has_sveltekit_app_result;
	}

	if (!(await fs_exists(svelte_config.lib_path))) {
		return {ok: false, message: `no SvelteKit lib directory found at ${svelte_config.lib_path}`};
	}

	if (!package_json_has_dependency(dep_name, package_json)) {
		return {
			ok: false,
			message: `no dependency found in package.json for ${dep_name}`,
		};
	}

	return {ok: true};
};

export const sveltekit_sync = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI,
	pm_cli = PM_CLI_DEFAULT, // TODO source from config when possible, is just needed for error messages
): Promise<void> => {
	const result = await spawn_cli(sveltekit_cli, ['sync']);
	if (!result) {
		throw new TaskError(
			`Failed to find SvelteKit CLI \`${to_cli_name(sveltekit_cli)}\`, do you need to run \`${pm_cli} install\`?`,
		);
	} else if (!result.ok) {
		throw new TaskError(`Failed ${to_cli_name(sveltekit_cli)} sync`);
	}
};

// TODO maybe this shouldn't exist, instead error if `package.json` has SvelteKit but it's not found (with install message above)
/**
 * If the SvelteKit CLI is found and its `.svelte-kit` directory is not, run `svelte-kit sync`.
 */
export const sveltekit_sync_if_available = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI,
): Promise<void> => {
	const found_sveltekit_cli =
		typeof sveltekit_cli === 'string' ? await find_cli(sveltekit_cli) : sveltekit_cli;
	if (found_sveltekit_cli) {
		return sveltekit_sync(found_sveltekit_cli);
	}
};

/**
 * If the SvelteKit CLI is found and its `.svelte-kit` directory is not, run `svelte-kit sync`.
 */
export const sveltekit_sync_if_obviously_needed = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI,
): Promise<void> => {
	if (await fs_exists(SVELTEKIT_DEV_DIRNAME)) {
		return;
	}
	const found_sveltekit_cli =
		typeof sveltekit_cli === 'string' ? await find_cli(sveltekit_cli) : sveltekit_cli;
	if (!found_sveltekit_cli) {
		return;
	}
	return sveltekit_sync(found_sveltekit_cli);
};

/**
 * Options to the SvelteKit packaging CLI.
 * @see https://svelte.dev/docs/kit/packaging#options
 */
export interface SveltePackageOptions {
	/**
	 * Watch files in src/lib for changes and rebuild the package
	 */
	watch?: boolean;
	/**
	 * Alias for `watch`.
	 */
	w?: boolean;
	/**
	 * The input directory which contains all the files of the package.
	 * Defaults to src/lib
	 */
	input?: string;
	/**
	 * Alias for `input`.
	 */
	i?: string;
	/**
	 * The output directory where the processed files are written to.
	 * Your package.json's exports should point to files inside there,
	 * and the files array should include that folder.
	 * Defaults to dist
	 */
	output?: string;
	/**
	 * Alias for `output`.
	 */
	o?: string;
	/**
	 * Whether or not to create type definitions (d.ts files).
	 * We strongly recommend doing this as it fosters ecosystem library quality.
	 * Defaults to true
	 */
	types?: boolean;
	/**
	 * Alias for `types`.
	 */
	t?: boolean;
	/**
	 * The path to a tsconfig or jsconfig.
	 * When not provided, searches for the next upper tsconfig/jsconfig in the workspace path.
	 */
	tsconfig?: string;
}

export const run_svelte_package = async (
	package_json: PackageJson,
	options: SveltePackageOptions | undefined,
	cli: string | Cli,
	log: Logger,
	pm_cli: string,
): Promise<void> => {
	const has_sveltekit_library_result = await has_sveltekit_library(package_json);
	if (!has_sveltekit_library_result.ok) {
		throw new TaskError(
			'Failed to find SvelteKit library: ' + has_sveltekit_library_result.message,
		);
	}
	const cli_name = typeof cli === 'string' ? cli : cli.name;
	const found_svelte_package_cli = cli === cli_name ? await find_cli(cli) : (cli as Cli);
	if (found_svelte_package_cli?.kind !== 'local') {
		throw new TaskError(
			`Failed to find SvelteKit packaging CLI \`${cli_name}\`, do you need to run \`${pm_cli} install\`?`,
		);
	}
	const serialized_args = args_serialize({
		...options,
		...to_forwarded_args(cli_name),
	});
	await spawn_cli(found_svelte_package_cli, serialized_args, log);
};

//
/**
 * Map an import specifier with the SvelteKit aliases.
 */
export const map_sveltekit_aliases = (
	specifier: string,
	aliases: Array<[string, string]>,
): string => {
	let path = specifier;
	for (const [from, to] of aliases) {
		if (path.startsWith(from)) {
			path = join(process.cwd(), to, path.substring(from.length));
			break;
		}
	}
	return path;
};
