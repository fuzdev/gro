import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { args_serialize } from '@fuzdev/fuz_util/args.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';

import { to_forwarded_args } from './args.ts';
import { find_cli, spawn_cli, to_cli_name, type Cli } from './cli.ts';
import {
	PM_CLI_DEFAULT,
	SVELTE_PACKAGE_DEP_NAME,
	SVELTEKIT_CLI,
	SVELTEKIT_DEP_NAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_TSCONFIG_PATH,
	SVELTEKIT_TSCONFIG_SPECIFIER,
	TSCONFIG_FILENAME,
	LIB_PATH
} from './constants.ts';
import { package_json_has_dependency } from './package_json.ts';
import { TaskError } from './task.ts';

/**
 * Detected from `package.json` rather than the Svelte config,
 * because reading the config costs a full Vite config resolution.
 * Peer deps don't count - a package peered on SvelteKit is built to work with one,
 * not to be one, and counting them would run `vite build` over a library that has no app.
 */
export const has_sveltekit_app = (
	package_json: PackageJson
): Result<object, { message: string }> => {
	if (!package_json_has_dependency(SVELTEKIT_DEP_NAME, package_json, false)) {
		return { ok: false, message: `no dependency found in package.json for ${SVELTEKIT_DEP_NAME}` };
	}
	return { ok: true };
};

export const has_sveltekit_library = async (
	package_json: PackageJson
): Promise<Result<object, { message: string }>> => {
	const has_sveltekit_app_result = has_sveltekit_app(package_json);
	if (!has_sveltekit_app_result.ok) {
		return has_sveltekit_app_result;
	}

	// Checked before the lib directory because it's the cheaper of the two and it's what
	// Peer deps don't count here, for the same reason as `has_sveltekit_app`.
	if (!package_json_has_dependency(SVELTE_PACKAGE_DEP_NAME, package_json, false)) {
		return {
			ok: false,
			message: `no dependency found in package.json for ${SVELTE_PACKAGE_DEP_NAME}`
		};
	}

	if (!(await fs_exists(LIB_PATH))) {
		return { ok: false, message: `no SvelteKit lib directory found at ${LIB_PATH}` };
	}

	return { ok: true };
};

export const sveltekit_sync = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI,
	pm_cli = PM_CLI_DEFAULT // TODO source from config when possible, is just needed for error messages
): Promise<void> => {
	const result = await spawn_cli(sveltekit_cli, ['sync']);
	if (!result) {
		throw new TaskError(
			`Failed to find SvelteKit CLI \`${to_cli_name(sveltekit_cli)}\`, do you need to run \`${
				pm_cli
			} install\`?`
		);
	} else if (!result.ok) {
		throw new TaskError(`Failed ${to_cli_name(sveltekit_cli)} sync`);
	}
};

// TODO maybe this shouldn't exist, instead error if `package.json` has SvelteKit but it's not found (with install message above)
/**
 * If the SvelteKit CLI is found, run `svelte-kit sync`.
 */
export const sveltekit_sync_if_available = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI
): Promise<void> => {
	const found_sveltekit_cli =
		typeof sveltekit_cli === 'string' ? await find_cli(sveltekit_cli) : sveltekit_cli;
	if (found_sveltekit_cli) {
		return sveltekit_sync(found_sveltekit_cli);
	}
};

/**
 * If the SvelteKit CLI is found and either its `.svelte-kit` directory
 * or its generated `$app/tsconfig` is not, run `svelte-kit sync`.
 * The tsconfig is checked because it lives in `node_modules`, which reinstalls wipe.
 */
export const sveltekit_sync_if_obviously_needed = async (
	sveltekit_cli: string | Cli = SVELTEKIT_CLI
): Promise<void> => {
	if ((await fs_exists(SVELTEKIT_DEV_DIRNAME)) && (await fs_exists(SVELTEKIT_TSCONFIG_PATH))) {
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
 * Whether `dir`'s `tsconfig.json` extends SvelteKit's `$app/tsconfig` while
 * the generated `node_modules/$app/tsconfig.json` is missing - the project was
 * never synced, or a reinstall wiped `node_modules`. TypeScript doesn't fail on
 * an unresolved `extends`, so tools reading such a tsconfig silently run on
 * default compiler options and read external types as `any`.
 *
 * Reads the tsconfig as JSONC with TypeScript's own parser and accepts a string
 * or array `extends`. Only `dir`'s own `tsconfig.json` is checked, not configs
 * it extends. A missing tsconfig, or one TypeScript's tolerant parser recovers
 * no `extends` from, returns `false`, left to whatever reads it next.
 *
 * @param dir - absolute path to the project root
 * @returns `true` when the project needs `svelte-kit sync`
 */
export const sveltekit_tsconfig_is_unsynced = async (dir: string): Promise<boolean> => {
	let contents: string;
	try {
		contents = await readFile(join(dir, TSCONFIG_FILENAME), 'utf-8');
	} catch {
		return false;
	}
	// lazy because this module loads on every `gro` invocation and few need TypeScript
	const { default: ts } = await import('typescript');
	const { config } = ts.parseConfigFileTextToJson(TSCONFIG_FILENAME, contents);
	const extended: unknown = config?.extends;
	const specifiers = Array.isArray(extended) ? extended : [extended];
	const extends_sveltekit = specifiers.some(
		(s) => s === SVELTEKIT_TSCONFIG_SPECIFIER || s === SVELTEKIT_TSCONFIG_SPECIFIER + '.json'
	);
	if (!extends_sveltekit) return false;
	return !(await fs_exists(join(dir, SVELTEKIT_TSCONFIG_PATH)));
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
	 * Your `package.json`'s exports should point to files inside there,
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
	pm_cli: string
): Promise<void> => {
	const has_sveltekit_library_result = await has_sveltekit_library(package_json);
	if (!has_sveltekit_library_result.ok) {
		throw new TaskError(
			'Failed to find SvelteKit library: ' + has_sveltekit_library_result.message
		);
	}
	const cli_name = typeof cli === 'string' ? cli : cli.name;
	const found_svelte_package_cli = cli === cli_name ? await find_cli(cli) : (cli as Cli);
	if (found_svelte_package_cli?.kind !== 'local') {
		throw new TaskError(
			`Failed to find SvelteKit packaging CLI \`${cli_name}\`, do you need to run \`${
				pm_cli
			} install\`?`
		);
	}
	const serialized_args = args_serialize({
		...options,
		...to_forwarded_args(cli_name)
	});
	await spawn_cli(found_svelte_package_cli, serialized_args, log);
};
