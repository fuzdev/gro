import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';

import { has_forwarded_flag } from './task_args.ts';
import {
	find_local_cli,
	spawn_cli,
	spawn_result_is_ok,
	to_spawn_result_message,
	type CliFinder
} from './cli.ts';
import {
	SVELTE_PACKAGE_CLI,
	SVELTE_PACKAGE_DEP_NAME,
	SVELTEKIT_CLI,
	SVELTEKIT_DEP_NAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_TSCONFIG_PATH,
	SVELTEKIT_TSCONFIG_SPECIFIER,
	TSCONFIG_FILENAME,
	LIB_PATH,
	NODE_MODULES_DIRNAME
} from './constants.ts';
import { package_json_has_dependency, package_json_load } from './package_json.ts';
import { TaskError } from './task_error.ts';
import type { FsPaths } from './fs_paths.ts';
import type { Spawn, SpawnProgram } from './spawn.ts';
import { posix_join } from './internal/posix_path.ts';

/**
 * Detected from `package.json` rather than the Svelte config,
 * because reading the config costs a full Vite config resolution.
 * Peer deps don't count - a package peered on SvelteKit is built to work with one,
 * not to be one, and counting them would run `vite build` over a library that has no app.
 */
export const has_sveltekit_app = (
	package_json: PackageJson
): Result<object, { message: string }> => {
	if (!package_json_has_dependency(SVELTEKIT_DEP_NAME, package_json)) {
		return { ok: false, message: `no dependency found in package.json for ${SVELTEKIT_DEP_NAME}` };
	}
	return { ok: true };
};

/**
 * Whether the project's `package.json` declares a SvelteKit app, as `has_sveltekit_app` detects one.
 * Read without the schema, as only its dependencies matter, and one that can't be read is no app.
 * @param fs_paths - reads the project's `package.json`
 * @param log - reports a `package.json` that exists but can't be read or parsed; with none, stays silent
 * @returns whether the project is a SvelteKit app
 */
export const project_has_sveltekit_app = async (
	fs_paths: FsPaths,
	log?: Logger
): Promise<boolean> =>
	has_sveltekit_app(await package_json_load(fs_paths, undefined, undefined, false, log)).ok;

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
	if (!package_json_has_dependency(SVELTE_PACKAGE_DEP_NAME, package_json)) {
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

/**
 * The SvelteKit CLI's environment: `svelte-kit sync` loads the project's Vite config,
 * code whose environment Gro can't know.
 */
export const SVELTEKIT_PROGRAM_OPTIONS = { pass_env: 'all' } as const;

/**
 * Runs `svelte-kit sync`, with nothing checking that the project uses SvelteKit.
 * Callers gate on `project_has_sveltekit_app` unless something else already establishes it.
 * @param finder - finds the CLI, with its environment
 * @throws `TaskError` if the CLI isn't found or fails
 */
export const sveltekit_sync = async (finder: CliFinder): Promise<void> => {
	const found_sveltekit_cli = finder.find(SVELTEKIT_CLI);
	if (!found_sveltekit_cli) {
		throw new TaskError(
			`Failed to find SvelteKit CLI \`${SVELTEKIT_CLI}\`, do you need to run \`npm install\`?`
		);
	}
	await run_sveltekit_sync(found_sveltekit_cli);
};

const run_sveltekit_sync = async (found_sveltekit_cli: SpawnProgram): Promise<void> => {
	const result = await spawn_cli(found_sveltekit_cli, ['sync']);
	if (!spawn_result_is_ok(result)) {
		throw new TaskError(`Failed ${SVELTEKIT_CLI} sync`);
	}
};

/**
 * What the builtins that read SvelteKit's generated files run first, `gen`, `lint`, and `check`
 * (`typecheck` runs a full sync): `svelte-kit sync` for a project `has_sveltekit_app` detects whose
 * `.svelte-kit` directory or generated `$app/tsconfig` is missing, the tsconfig checked because
 * it lives in `node_modules`, which reinstalls wipe. A project with both pays only their checks.
 * A CLI that isn't found skips rather than throws, so `gro sync --install` runs on a fresh clone,
 * and warns that the files weren't synced when the project has a `node_modules`,
 * an install missing SvelteKit, and stays quiet on a fresh clone with none,
 * which `gro sync --install` installs.
 * @param fs_paths - checks for the project's synced files and reads its `package.json`
 * @param finder - finds the CLI, with its environment
 * @param log - warns of a skip
 * @throws `TaskError` if the CLI fails
 */
export const sveltekit_sync_if_obviously_needed = async (
	fs_paths: FsPaths,
	finder: CliFinder,
	log: Pick<Logger, 'warn'>
): Promise<void> => {
	if (
		(await path_exists(fs_paths, SVELTEKIT_DEV_DIRNAME)) &&
		(await path_exists(fs_paths, SVELTEKIT_TSCONFIG_PATH))
	) {
		return;
	}
	if (!(await project_has_sveltekit_app(fs_paths))) return;
	const found_sveltekit_cli = finder.find(SVELTEKIT_CLI);
	if (found_sveltekit_cli) {
		await run_sveltekit_sync(found_sveltekit_cli);
	} else if (await path_exists(fs_paths, NODE_MODULES_DIRNAME)) {
		log.warn(
			`${SVELTEKIT_DEP_NAME} is declared but \`${SVELTEKIT_CLI}\` isn't in node_modules/.bin,` +
				" so SvelteKit's files weren't synced - run `npm i`"
		);
	}
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
	const tsconfig = await fs_paths.open_path(posix_join(dir, TSCONFIG_FILENAME));
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
	return !(await path_exists(fs_paths, posix_join(dir, SVELTEKIT_TSCONFIG_PATH)));
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
	 * The verbatim args forwarded to the CLI, from `to_forwarded_argv`.
	 */
	forwarded_argv: Array<string>;
	log: Logger;
}

/**
 * Runs `svelte-package` for a SvelteKit library, with any forwarded `-- svelte-package` args,
 * found only in the project's `node_modules/.bin`.
 * @param fs_paths - checks for the project's lib directory
 * @param spawn - finds and runs the CLI
 * @param options - the project's `package.json`, the CLI's options, and its forwarded args
 * @throws `TaskError` if the project isn't a SvelteKit library, the CLI isn't installed locally,
 * or `svelte-package` fails
 */
export const run_svelte_package = async (
	fs_paths: FsPaths,
	spawn: Spawn,
	{ package_json, svelte_package_options, forwarded_argv, log }: RunSveltePackageOptions
): Promise<void> => {
	const has_sveltekit_library_result = await has_sveltekit_library(fs_paths, package_json);
	if (!has_sveltekit_library_result.ok) {
		throw new TaskError(
			'Failed to find SvelteKit library: ' + has_sveltekit_library_result.message
		);
	}
	// a local install only, and it loads the project's Vite config, code whose environment Gro can't know
	const found_svelte_package_cli = find_local_cli(spawn, SVELTE_PACKAGE_CLI, { pass_env: 'all' });
	if (!found_svelte_package_cli) {
		throw new TaskError(
			`Failed to find SvelteKit packaging CLI \`${SVELTE_PACKAGE_CLI}\`, do you need to run \`npm install\`?`
		);
	}
	const serialized_args = [
		...serialize_svelte_package_options(svelte_package_options, forwarded_argv),
		...forwarded_argv
	];
	const result = await spawn_cli(found_svelte_package_cli, serialized_args, log);
	if (!spawn_result_is_ok(result)) {
		throw new TaskError(`\`${SVELTE_PACKAGE_CLI}\` failed: ${to_spawn_result_message(result)}`);
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
