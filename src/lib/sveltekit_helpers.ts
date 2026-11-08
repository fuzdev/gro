import { join } from 'node:path';
import { print_spawn_result } from '@fuzdev/fuz_util/process.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';

import { has_forwarded_flag, to_forwarded_argv } from './args.ts';
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
import type { FsPaths } from './fs_paths.ts';

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

/**
 * Whether the project is a SvelteKit library: a SvelteKit app, as `has_sveltekit_app` detects one,
 * with `@sveltejs/package` and the lib directory.
 * @param fs_paths - checks for the project's lib directory
 * @param package_json - the project's `package.json`
 */
export const has_sveltekit_library = async (
	fs_paths: FsPaths,
	package_json: PackageJson
): Promise<Result<object, { message: string }>> => {
	const has_sveltekit_app_result = has_sveltekit_app(package_json);
	if (!has_sveltekit_app_result.ok) {
		return has_sveltekit_app_result;
	}

	// checked before the lib directory because it's the cheaper of the two and it's what
	// distinguishes a library from an app; peer deps don't count, as in `has_sveltekit_app`
	if (!package_json_has_dependency(SVELTE_PACKAGE_DEP_NAME, package_json, false)) {
		return {
			ok: false,
			message: `no dependency found in package.json for ${SVELTE_PACKAGE_DEP_NAME}`
		};
	}

	if (!(await path_exists(fs_paths, LIB_PATH))) {
		return { ok: false, message: `no SvelteKit lib directory found at ${LIB_PATH}` };
	}

	return { ok: true };
};

export interface SveltekitSyncOptions {
	/**
	 * The SvelteKit CLI's name or a found `Cli`.
	 * @default `SVELTEKIT_CLI`
	 */
	sveltekit_cli?: string | Cli;
	/**
	 * The package manager named in the install hint when the CLI isn't found.
	 * @default `PM_CLI_DEFAULT`
	 */
	pm_cli?: string; // TODO source from config when possible, is just needed for error messages
}

/**
 * Runs `svelte-kit sync`.
 * @throws `TaskError` if the CLI isn't found or fails
 */
export const sveltekit_sync = async ({
	sveltekit_cli = SVELTEKIT_CLI,
	pm_cli = PM_CLI_DEFAULT
}: SveltekitSyncOptions = {}): Promise<void> => {
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
		return sveltekit_sync({ sveltekit_cli: found_sveltekit_cli });
	}
};

/**
 * If the SvelteKit CLI is found and either the project's `.svelte-kit` directory
 * or its generated `$app/tsconfig` is not, run `svelte-kit sync`.
 * The tsconfig is checked because it lives in `node_modules`, which reinstalls wipe.
 * @param fs_paths - checks for the project's synced files
 * @param sveltekit_cli - the SvelteKit CLI's name or a found `Cli`
 */
export const sveltekit_sync_if_obviously_needed = async (
	fs_paths: FsPaths,
	sveltekit_cli: string | Cli = SVELTEKIT_CLI
): Promise<void> => {
	if (
		(await path_exists(fs_paths, SVELTEKIT_DEV_DIRNAME)) &&
		(await path_exists(fs_paths, SVELTEKIT_TSCONFIG_PATH))
	) {
		return;
	}
	const found_sveltekit_cli =
		typeof sveltekit_cli === 'string' ? await find_cli(sveltekit_cli) : sveltekit_cli;
	if (!found_sveltekit_cli) {
		return;
	}
	return sveltekit_sync({ sveltekit_cli: found_sveltekit_cli });
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
 * @param fs_paths - opens `dir`'s files
 * @param dir - the project root, absolute or relative to the project of `fs_paths`
 * @returns `true` when the project needs `svelte-kit sync`
 * @throws `FsError` - when reading either file fails other than by its absence
 */
export const sveltekit_tsconfig_is_unsynced = async (
	fs_paths: FsPaths,
	dir: string
): Promise<boolean> => {
	const tsconfig = await fs_paths.open_path(join(dir, TSCONFIG_FILENAME));
	const contents = await tsconfig.dir.read_text_if_exists(tsconfig.path);
	if (contents === null) return false;
	// lazy because this module loads on every `gro` invocation and few need TypeScript
	const { default: ts } = await import('typescript');
	const { config } = ts.parseConfigFileTextToJson(TSCONFIG_FILENAME, contents);
	const extended: unknown = config?.extends;
	const specifiers = Array.isArray(extended) ? extended : [extended];
	const extends_sveltekit = specifiers.some(
		(s) => s === SVELTEKIT_TSCONFIG_SPECIFIER || s === SVELTEKIT_TSCONFIG_SPECIFIER + '.json'
	);
	if (!extends_sveltekit) return false;
	return !(await path_exists(fs_paths, join(dir, SVELTEKIT_TSCONFIG_PATH)));
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

export interface RunSveltePackageOptions {
	package_json: PackageJson;
	/**
	 * The options serialized as `svelte-package` flags, each dropped when the forwarded args name it.
	 */
	svelte_package_options?: SveltePackageOptions;
	/**
	 * The `svelte-package` CLI's name or a found `Cli`.
	 */
	svelte_package_cli: string | Cli;
	log: Logger;
	/**
	 * The package manager named in the install hint when the CLI isn't found.
	 */
	pm_cli: string;
}

/**
 * Runs `svelte-package` for a SvelteKit library, with any forwarded `-- svelte-package` args.
 * @param fs_paths - checks for the project's lib directory
 * @param options - the project's `package.json`, the CLI and its options, and the install hint
 * @throws `TaskError` if the project isn't a SvelteKit library, the CLI isn't installed locally,
 * or `svelte-package` fails
 */
export const run_svelte_package = async (
	fs_paths: FsPaths,
	{
		package_json,
		svelte_package_options,
		svelte_package_cli: cli,
		log,
		pm_cli
	}: RunSveltePackageOptions
): Promise<void> => {
	const has_sveltekit_library_result = await has_sveltekit_library(fs_paths, package_json);
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
	const forwarded_argv = to_forwarded_argv(cli_name);
	const serialized_args = [
		...serialize_svelte_package_options(svelte_package_options, forwarded_argv),
		...forwarded_argv
	];
	const spawned = await spawn_cli(found_svelte_package_cli, serialized_args, log);
	if (!spawned?.ok) {
		throw new TaskError(
			`\`${cli_name}\` failed` + (spawned ? `: ${print_spawn_result(spawned)}` : '')
		);
	}
};

/**
 * The long names of the boolean short aliases in `SveltePackageOptions`.
 */
const SVELTE_PACKAGE_BOOLEAN_ALIASES: Record<string, string | undefined> = {
	w: 'watch',
	t: 'types'
};

/**
 * Serializes the configured `svelte-package` options by their spelling,
 * a `false` boolean as `--no-x` with a short alias's long name,
 * as `svelte-package` negates only long names (`--no-watch`, not `--no-w`),
 * dropping each option that the verbatim `forwarded_argv` names, which overrides it.
 * @param options - the configured options
 * @param forwarded_argv - the verbatim args forwarded to `svelte-package`
 * @returns the args for the configured options, to precede `forwarded_argv`
 */
const serialize_svelte_package_options = (
	options: SveltePackageOptions | undefined,
	forwarded_argv: Array<string>
): Array<string> => {
	const serialized: Array<string> = [];
	if (!options) return serialized;
	for (const [key, value] of Object.entries(options)) {
		if (value === undefined || has_forwarded_flag(forwarded_argv, key)) continue;
		if (value === false) {
			serialized.push('--no-' + (SVELTE_PACKAGE_BOOLEAN_ALIASES[key] ?? key));
			continue;
		}
		const name = (key.length === 1 ? '-' : '--') + key;
		if (value === true) serialized.push(name);
		else serialized.push(name, String(value));
	}
	return serialized;
};

// whether anything is at an absolute or project-relative path, following symlinks
const path_exists = async (fs_paths: FsPaths, path: string): Promise<boolean> => {
	const opened = await fs_paths.open_path(path);
	return opened.dir.exists(opened.path);
};
