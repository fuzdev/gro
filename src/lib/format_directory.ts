import {args_serialize, type Args} from '@fuzdev/fuz_util/args.ts';
import type {Logger} from '@fuzdev/fuz_util/log.ts';
import type {SpawnResult} from '@fuzdev/fuz_util/process.ts';

import {spawn_cli, to_cli_name, type Cli} from './cli.ts';
import {
	GITHUB_DIRNAME,
	README_FILENAME,
	SVELTE_CONFIG_FILENAME,
	VITE_CONFIG_FILENAME,
	TSCONFIG_FILENAME,
	GRO_CONFIG_FILENAME,
	PM_CLI_DEFAULT,
	PRETTIER_CLI_DEFAULT,
} from './constants.ts';
import {paths} from './paths.ts';

const EXTENSIONS_DEFAULT = 'ts,js,json,svelte,html,css,md,yml';
const ROOT_PATHS_DEFAULT = `${[
	README_FILENAME,
	GRO_CONFIG_FILENAME,
	SVELTE_CONFIG_FILENAME,
	VITE_CONFIG_FILENAME,
	TSCONFIG_FILENAME,
	GITHUB_DIRNAME,
].join(',')}/**/*`;

/**
 * Formats files on the filesystem.
 * When `patterns` is provided, formats those specific files/patterns.
 * Otherwise formats `dir` with default extensions, plus root files if `dir` is `paths.source`.
 * This is separated from `./format_file` to avoid importing all of the `prettier` code
 * inside modules that import this one. (which has a nontrivial cost)
 */
export const format_directory = async (
	log: Logger,
	dir: string,
	check = false,
	extensions = EXTENSIONS_DEFAULT,
	root_paths = ROOT_PATHS_DEFAULT,
	prettier_cli: string | Cli = PRETTIER_CLI_DEFAULT,
	pm_cli: string = PM_CLI_DEFAULT,
	additional_args?: Args,
	patterns?: Array<string>,
): Promise<SpawnResult> => {
	const forwarded_args = {...additional_args};
	if (forwarded_args.check === undefined && forwarded_args.write === undefined) {
		forwarded_args[check ? 'check' : 'write'] = true;
	}
	const serialized_args = args_serialize(forwarded_args);
	if (patterns?.length) {
		serialized_args.push(...patterns);
	} else {
		serialized_args.push(`${dir}**/*.{${extensions}}`);
		if (dir === paths.source) {
			serialized_args.push(`${paths.root}{${root_paths}}`);
		}
	}
	const spawned = await spawn_cli(prettier_cli, serialized_args, log);
	if (!spawned)
		throw Error(
			`failed to find \`${to_cli_name(prettier_cli)}\` CLI locally or globally, do you need to run \`${pm_cli} install\`?`,
		);
	return spawned;
};
